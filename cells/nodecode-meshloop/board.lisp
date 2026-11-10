;;;; board.lisp --- meshloop run events projected onto a nodecode Board (#137).
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; Optional, off by default (meshloop.board). ONE-WAY: meshloop's event log,
;;;; read through `meshloop events --json', becomes pointer posts on one board
;;;; per repository. Nothing here reads a Board post, and nothing here runs a
;;;; meshloop verb that writes: a post that says "accept" is text on a board,
;;;; never acceptance (ADR 0038, invariants 1, 5 and 6).
;;;;
;;;; When it runs. Only inside a /meshloop command the operator typed:
;;;; `/meshloop board-sync [GRAPH]', and, while meshloop.board_after_commands is
;;;; on, after /meshloop run, resume, accept, cancel and status. No timer, no
;;;; hook point, no thread: a transition reaches the board at the next such
;;;; command (ADR 0022 keeps meshloop daemonless; the projection adds no daemon).
;;;; Eval verbs never project, so a model turn does not post.
;;;;
;;;; The board. `meshloop-' and 12 hex digits of the SHA-256 of the repository
;;;; root's real path: the same repository always names the same board, and a
;;;; session can sit on only one. The session that typed the command is put on
;;;; it (board:join), and every post is that session's (board:post).
;;;;
;;;; The post. A fixed template of meshloop's own words: the graph id, the
;;;; event's id in meshloop's log, the node, its new state, the state it left,
;;;; the event's name and the attempt's worktree. No reason, note, description,
;;;; diff, log or worker output, so no text a run produced. Every field is
;;;; checked or escaped so it can hold no `@': a post mentions nobody, unless
;;;; meshloop.wake_on_gates is on, and then only meshloop.supervisor_session, on
;;;; a gate (AwaitingPlanReview, AwaitingReview, Blocked, Failed) that is still
;;;; the node's state. A post fits board.post_chars: the evidence path is
;;;; shortened from the left first.
;;;;
;;;; The cursor. Per repository and graph, the id of the last event posted and
;;;; the last plan state posted, in a file of this cell's own under the nodecode
;;;; home (meshloop/board/<store>/<board>.cursor), never in meshloop's store. It
;;;; moves after each post, so a replay, a resume or a sync after an interrupted
;;;; one posts each event once. The event id in the text keeps two posts
;;;; distinct for the Board's 60-second same-words refusal, and that refusal is
;;;; read as "already posted": the one window left is a crash between a post and
;;;; its cursor write more than 60 seconds before the next sync.

(in-package #:nodecode-meshloop)

;;; --- words --------------------------------------------------------------------

(defparameter +gate-states+ '("AwaitingPlanReview" "AwaitingReview" "Blocked" "Failed")
  "The states that may mention the supervising session, when wake_on_gates is on.")

(defparameter +board-cursor-header+ "nodecode-meshloop board cursor 1"
  "The first line of a cursor file.")

(defparameter *board-batch* 200
  "The most posts one graph gets in one sync; the rest wait for the next.")

(defparameter *board-default-chars* 1000
  "board.post_chars when the board cell does not say.")

(defun gate-p (state)
  "Whether STATE is one a supervising session may be woken for."
  (and (member state +gate-states+ :test #'equal) t))

(defun safe-word (text)
  "TEXT when it is 1 to 64 ASCII letters and digits, as meshloop's state and
event names are; else ?."
  (if (and (stringp text) (<= 1 (length text) 64)
           (every (lambda (c) (and (< (char-code c) 128) (alphanumericp c))) text))
      text
      "?"))

(defun safe-path (text)
  "TEXT as one line that cannot mention anyone: control characters dropped,
@ written %40."
  (with-output-to-string (out)
    (loop for c across (or text "")
          do (cond ((< (char-code c) 32))
                   ((char= c #\@) (write-string "%40" out))
                   (t (write-char c out))))))

(defun relative-to (path root)
  "PATH under ROOT, the repository root, as a path relative to it; else PATH."
  (let ((root (if (suffix-p "/" root) root (format nil "~a/" root))))
    (if (and (> (length path) (length root)) (string= root path :end2 (length root)))
        (subseq path (length root))
        path)))

(defun supervisor-id-p (text)
  "Whether TEXT can be a session id to mention: 1 to 64 of ASCII letters,
digits, - and _, as the Board reads an @ID."
  (and (stringp text) (<= 1 (length text) 64)
       (every (lambda (c) (or (and (< (char-code c) 128) (alphanumericp c)) (find c "-_"))) text)))

(defun mention-of (state current wake supervisor)
  "The session a post about STATE names, or NIL: only with WAKE on, a
SUPERVISOR that is a session id, a gate STATE, and CURRENT (still the node's
state, so history never wakes anyone)."
  (and wake current (gate-p state) (supervisor-id-p supervisor) supervisor))

(defun fit-post (head evidence mention limit)
  "HEAD, then `; evidence EVIDENCE' and ` @MENTION', in at most LIMIT
characters: EVIDENCE is shortened from the left first, then left out."
  (let* ((tail (if mention (format nil " @~a" mention) ""))
         (room (- limit (length head) (length "; evidence ") (length tail)))
         (evidence (cond ((null evidence) "none")
                         ((<= (length evidence) room) evidence)
                         ((>= room 8) (concatenate 'string "..." (subseq evidence (- (length evidence) (- room 3)))))
                         (t nil)))
         (text (if evidence
                   (format nil "~a; evidence ~a~a" head evidence tail)
                   (format nil "~a~a" head tail))))
    (if (> (length text) limit) (subseq text 0 limit) text)))

(defun event-post (graph seq task to from event evidence mention limit)
  "The post for meshloop event SEQ of GRAPH: node TASK now TO, from FROM by
EVENT, EVIDENCE its attempt's worktree or NIL, naming MENTION or nobody."
  (fit-post (format nil "meshloop ~a event ~d: node #~d ~a (from ~a, ~a)"
                    graph seq task (safe-word to) (safe-word from) (safe-word event))
            (and evidence (safe-path evidence)) mention limit))

(defun plan-post (graph state change mention limit)
  "The post for GRAPH's plan reaching STATE, its CHANGEth plan state posted."
  (fit-post (format nil "meshloop ~a plan ~d: ~a" graph change (safe-word state))
            nil mention limit))

;;; --- the board's name --------------------------------------------------------------

(defun board-name-of (root)
  "The board of the repository at ROOT: meshloop- and 12 hex digits of the
SHA-256 of ROOT, its real path without a trailing /."
  (let ((root (string-right-trim "/" root)))
    (format nil "meshloop-~a" (subseq (nlk:sha256-text root) 7 19))))

(defun real-root (directory)
  "DIRECTORY's real path as a string, links resolved."
  (let ((true (ignore-errors (truename (uiop:ensure-directory-pathname directory)))))
    (uiop:native-namestring (or true directory))))

;;; --- the cursor ------------------------------------------------------------------
;;; A text file: the header, `board NAME', `repository ROOT', then one line a
;;; graph: `graph ID SEQ PLAN-STATE PLAN-POSTS'. Graph ids, states and numbers
;;; hold no blank, so a line is its words.

(defun cursor-text (board root entries)
  "The cursor file for BOARD and ROOT over ENTRIES, each (GRAPH SEQ PLAN POSTS)."
  (with-output-to-string (out)
    (format out "~a~%board ~a~%repository ~a~%" +board-cursor-header+ board root)
    (loop for (graph seq plan posts) in (sort (copy-list entries) #'string< :key #'first)
          do (format out "graph ~a ~d ~a ~d~%" graph seq (or plan "-") posts))))

(defun parse-cursor (text)
  "The entries of cursor file TEXT, each (GRAPH SEQ PLAN POSTS), PLAN NIL for
none; a refusal when TEXT is no cursor file."
  (let ((lines (remove "" (uiop:split-string (or text "") :separator '(#\Newline)) :test #'string=)))
    (unless (equal (first lines) +board-cursor-header+)
      (fail "the board cursor file is not one this cell wrote; move it away to start over (every event is posted again)"))
    (loop for line in (rest lines)
          for words = (nlk:split-words line)
          when (equal (first words) "graph")
            collect (destructuring-bind (&optional graph seq plan posts &rest more) (rest words)
                      (unless (and graph (graph-id-p graph) seq (every #'digit-char-p seq) (plusp (length seq))
                                   plan posts (every #'digit-char-p posts) (plusp (length posts)) (null more))
                        (fail "the board cursor file has a line that is no cursor: ~s" (nlk:one-line line :cap 80)))
                      (list graph (parse-integer seq) (if (string= plan "-") nil plan) (parse-integer posts))))))

(defun cursor-entry (entries graph)
  "GRAPH's (GRAPH SEQ PLAN POSTS) among ENTRIES, or a fresh one."
  (or (find graph entries :key #'first :test #'string=) (list graph 0 nil 0)))

(defun with-entry (entries entry)
  "ENTRIES with ENTRY in place of its graph's."
  (cons entry (remove (first entry) entries :key #'first :test #'string=)))

(defun store-key ()
  "12 hex digits naming the organism's open store, as the Board names its
folder, so an ephemeral store keeps cursors of its own."
  (let ((store (find-symbol "*STORE*" "NLK")))
    (or (ignore-errors
         (and store (boundp store) (symbol-value store)
              (subseq (nlk:sha256-text (namestring (nlk:store-path (symbol-value store)))) 7 19)))
        "store")))

(defun cursor-path (board)
  "Where BOARD's cursor file lives: this cell's own state under the home."
  (nlk:home (format nil "meshloop/board/~a/~a.cursor" (store-key) board)))

(defun read-cursor (board)
  "BOARD's cursor entries, none when there is no file."
  (let ((path (cursor-path board)))
    (if (probe-file path)
        (parse-cursor (uiop:read-file-string path :external-format :utf-8))
        '())))

(defun write-cursor (board root entries)
  "Write BOARD's cursor file whole, through one rename."
  (nlk:write-file-atomically (cursor-path board) (cursor-text board root entries)
                             :mode #o600 :directory-mode #o700))

;;; --- the board cell, an optional peer ---------------------------------------------
;;; Only the two exported verbs are called, by name at run time, so this cell
;;; loads and runs without the board cell. Neither reads a post.

(defun board-verb (name)
  "The board cell's exported verb NAME (JOIN or POST), or a refusal."
  (let* ((package (find-package "NODECODE-BOARD"))
         (symbol (and package (find-symbol name package))))
    (unless (and symbol (fboundp symbol))
      (fail "the nodecode-board cell is not loaded: it ships with nodecode; switch it on in /cells, or set meshloop.board to false"))
    (symbol-function symbol)))

(defun board-chars ()
  "board.post_chars, as the running board cell reads it, else 1000."
  (let* ((package (find-package "NODECODE-BOARD"))
         (setting (and package (find-symbol "SETTING" package)))
         (chars (and setting (fboundp setting) (ignore-errors (funcall setting :post-chars)))))
    (if (and (integerp chars) (plusp chars)) chars *board-default-chars*)))

(defun board-call (session name &rest arguments)
  "The board verb NAME on ARGUMENTS as SESSION's own call. => (values ANSWER
REFUSAL): the verb's answer, or NIL and its refusal's text."
  (let ((verb (board-verb name)))
    (when (nlk:on-standby-p session)
      ;; A shell that has not prompted stands by; the Board writes facts in its
      ;; session, which makes it one, as /board does.
      (nlk:materialize-standby-session session))
    (handler-case (let ((nlk:*scribe-session-id* session))
                    (values (apply verb arguments) nil))
      (error (condition) (values nil (nlk:one-line condition :cap (* 3 *text-cap*)))))))

(defun said-once-p (refusal)
  "Whether REFUSAL is the Board's same-words refusal: the post is there."
  (and refusal (search "a post is said once" refusal) t))

;;; --- one sync ----------------------------------------------------------------------

(defvar *board-lock* (sb-thread:make-mutex :name "nodecode-meshloop board")
  "One sync at a time, so two commands never post the same event.")

(defun latest-seqs (events)
  "An alist of task to the seq of its newest event among EVENTS."
  (let ((latest '()))
    (loop for event across events
          for task = (nlk:json-value event :integer "task")
          for seq = (nlk:json-value event :integer "seq")
          for entry = (assoc task latest)
          do (if entry (setf (cdr entry) (max (cdr entry) seq)) (push (cons task seq) latest)))
    latest))

(defun project-graph (session root board graph)
  "Post GRAPH's events past its cursor and its plan state when that moved.
=> one line saying what was posted, and whether a post woke anyone."
  (let* ((entries (read-cursor board))
         (entry (cursor-entry entries graph))
         (object (ask "events" (list "--graph" graph "--after" (princ-to-string (second entry))) session))
         (object (if (nlk:json-value object :boolean "ok")
                     (checked object "events")
                     (fail "~a" (refusal-of object))))
         (data (nlk:json-value object :object "data"))
         (events (nlk:json-array data "events"))
         (plan (nlk:json-value data :string "plan_state"))
         (latest (latest-seqs events))
         (wake (setting :wake-on-gates))
         (supervisor (setting :supervisor-session))
         (limit (board-chars))
         (posted 0) (woke '()) (stopped nil))
    (flet ((post (text)
             ;; T once TEXT is on the board, now or before; NIL with STOPPED set.
             ;; The Board answers a line for each session a post names: woke
             ;; it, or why not. Those are the Board's words, kept.
             (multiple-value-bind (answer refusal) (board-call session "POST" text)
               (cond (answer
                      (incf posted)
                      (dolist (line (rest (uiop:split-string answer :separator '(#\Newline))))
                        (when (plusp (length (nlk:trimmed line)))
                          (pushnew (nlk:one-line line :cap *text-cap*) woke :test #'string=)))
                      t)
                     ((said-once-p refusal) t)
                     (t (setf stopped refusal) nil))))
           (save (new)
             (setf entry new entries (with-entry entries new))
             (write-cursor board root entries)))
      (unless (equal plan (third entry))
        (let ((change (1+ (fourth entry))))
          (when (post (plan-post graph plan change (mention-of plan t wake supervisor) limit))
            (save (list graph (second entry) plan change)))))
      (unless stopped
        (loop for event across (subseq events 0 (min (length events) *board-batch*))
              for seq = (nlk:json-value event :integer "seq")
              for task = (nlk:json-value event :integer "task")
              for to = (nlk:json-value event :string "to")
              for current = (eql seq (cdr (assoc task latest)))
              for evidence = (nlk:json-value event :string "evidence_path")
              do (unless (post (event-post graph seq task to
                                           (nlk:json-value event :string "from")
                                           (nlk:json-value event :string "event")
                                           (and evidence (relative-to evidence root))
                                           (mention-of to current wake supervisor) limit))
                   (return))
                 (save (list graph seq (third entry) (fourth entry)))))
      (format nil "~a: ~d post~:p, cursor at event ~d~@[, ~d more wait for the next sync~]~:[~;~:*; the board said: ~{~a~^; ~}~]~@[; stopped: ~a~]"
              graph posted (second entry)
              (let ((left (count-if (lambda (e) (> (nlk:json-value e :integer "seq") (second entry))) events)))
                (and (null stopped) (plusp left) left))
              (reverse woke) stopped))))

(defun board-graphs (session-id typed)
  "The graphs a sync projects: TYPED, a list of graph ids the command named,
then the newest run and every run waiting for acceptance."
  (let* ((object (ask "status" '() session-id))
         (data (if (nlk:json-value object :boolean "ok")
                   (nlk:json-value object :object "data")
                   (fail "~a" (refusal-of object))))
         (graphs (copy-list typed)))
    (if (and data (nth-value 1 (gethash "graph_id" data)))
        (let ((data (nlk:json-value (checked object "status") :object "data")))
          (pushnew (nlk:json-value data :string "graph_id") graphs :test #'string=)
          (loop for graph across (nlk:json-array data "awaiting_acceptance")
                when (stringp graph) do (pushnew graph graphs :test #'string=)))
        (checked object "status_empty"))
    (remove-duplicates (reverse graphs) :test #'string= :from-end t)))

(defun board-sync (session-id typed)
  "Project the runs of SESSION-ID's repository onto its board. => the lines to
answer, each starting `meshloop board:'."
  (handler-case
      (progn
        (unless (setting :board)
          (fail "off (meshloop.board is false, the default); nothing is posted"))
        (sb-thread:with-mutex (*board-lock*)
          (let* ((root (real-root (session-directory session-id)))
                 (board (board-name-of root))
                 (graphs (board-graphs session-id typed))
                 (notes '()))
            (multiple-value-bind (joined refusal) (board-call session-id "JOIN" board)
              (unless joined (fail "cannot join ~a: ~a" board refusal)))
            (when (and (setting :wake-on-gates) (not (supervisor-id-p (setting :supervisor-session))))
              (push "wake_on_gates is on but supervisor_session names no session id, so no post mentions anyone" notes))
            (when (setting :board-workers)
              (push "board_workers is on, but nodecode-harness workers (#138) do not exist yet: no worker joins" notes))
            (format nil "meshloop board: ~a~:[: no managed runs to project~;~:*~{~%  ~a~}~]~{~%  note: ~a~}"
                    board
                    (loop for graph in graphs
                          collect (handler-case (project-graph session-id root board graph)
                                    (meshloop-error (condition)
                                      (format nil "~a: ~a" graph (meshloop-error-detail condition)))))
                    (reverse notes)))))
    (meshloop-error (condition)
      (format nil "meshloop board: ~a" (meshloop-error-detail condition)))
    (error (condition)
      (format nil "meshloop board: failed: ~a" (nlk:one-line condition :cap (* 3 *text-cap*))))))

;;; --- the commands -------------------------------------------------------------------

(defparameter +board-sync-usage+ "/meshloop board-sync [GRAPH]")

(defun board-sync-of (session-id text)
  "/meshloop board-sync [GRAPH]: project now."
  (handler-case
      (let ((words (nlk:split-words (or text ""))))
        (when (rest words) (fail "takes one graph id at most: ~a" +board-sync-usage+))
        (when (and words (not (graph-id-p (first words))))
          (fail "~s is no graph id: letters, digits and . _ : -, at most 128, not starting with -"
                (nlk:one-line (first words) :cap 40)))
        (board-sync session-id words))
    (meshloop-error (condition)
      (format nil "meshloop board: ~a" (meshloop-error-detail condition)))))

(defparameter +projecting-verbs+ '("run" "resume" "accept" "cancel" "status")
  "The /meshloop verbs after which the board is synced, when
meshloop.board_after_commands is on.")

(defun typed-graphs (verb words)
  "The graph ids the command VERB WORDS named: after --graph, or its first
positional word for status, resume and cancel."
  (let ((graphs '()) (positional nil))
    (loop while words
          do (let ((word (pop words)))
               (cond ((string= word "--graph")
                      (let ((next (pop words)))
                        (when (and next (graph-id-p next)) (push next graphs))))
                     ((member word '("--task" "--as" "--plan" "--into" "--reason") :test #'string=)
                      (pop words))
                     ((char= #\- (char word 0)))
                     ((null positional) (setf positional word)))))
    (when (and positional (graph-id-p positional)
               (member verb '("status" "resume" "cancel") :test #'string=))
      (push positional graphs))
    (nreverse graphs)))

(defun after-command (verb words answer session-id)
  "ANSWER, with the board's lines after it when VERB projects and the board
and board_after_commands are on; ANSWER untouched otherwise."
  (if (and (member verb +projecting-verbs+ :test #'string=)
           (stringp answer)
           (setting :board) (setting :board-after-commands))
      (format nil "~a~%~a" answer (board-sync session-id (typed-graphs verb words)))
      answer))
