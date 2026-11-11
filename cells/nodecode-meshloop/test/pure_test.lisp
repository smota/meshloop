;;;; pure_test.lisp --- an offline check of the cell under a plain SBCL.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;;   sbcl --script cells/nodecode-meshloop/test/pure_test.lisp
;;;;
;;;; No nodecode, no socket, no model. It reads every form of package.lisp,
;;;; cell.lisp, gates.lisp, board.lisp and drive.lisp (a reader error, an unbalanced
;;;; parenthesis, fails here), compiles every DEFUN, DEFPARAMETER and DEFVAR
;;;; among them against stand-in NLK and NLE packages (a full compiler warning,
;;;; such as a call with the wrong number of arguments, fails here), and then
;;;; tests the functions that need nothing of nodecode: the :tool veto's
;;;; reading of a call, the argument checks of the commands that write
;;;; (identity, task, graph, branch, plan path, flags), and the Board
;;;; projection's pure parts (board name, post template, mentions, cursor file)
;;;; with a source check that it is one-way (#137). The load test
;;;; (load_test.py) is what runs the cell in nodecode. Optional: neither
;;;; `cargo test' nor `xtask check' runs this. Exit status 0 means every check held.

(require :asdf) ; the cell reads uiop and asdf, as nodecode's image has them

(defpackage #:nodecode.kernel (:use #:cl) (:nicknames #:nlk))
(defpackage #:nodecode.evolved (:use #:cl) (:nicknames #:nle))

(defpackage #:pure-test (:use #:cl))
(in-package #:pure-test)

(defparameter *cell* (merge-pathnames "../" (make-pathname :name nil :type nil
                                                           :defaults *load-truename*)))
(defparameter *files* '("package" "cell" "gates" "board" "drive"))
(defparameter *failures* 0)
(defparameter *checks* 0)

(defun check (name ok &optional detail)
  (incf *checks*)
  (unless ok (incf *failures*))
  (format t "[~:[FAIL~;pass~]] ~a~@[: ~a~]~%" ok name (and (not ok) detail)))

(defun file-text-at (path)
  (with-open-file (in path :external-format :utf-8)
    (let ((text (make-string (file-length in))))
      (subseq text 0 (read-sequence text in)))))

(defun file-text (name)
  (with-open-file (in (merge-pathnames (format nil "~a.lisp" name) *cell*) :external-format :utf-8)
    (let ((text (make-string (file-length in))))
      (subseq text 0 (read-sequence text in)))))

;;; --- stand-ins: every nlk: and nle: symbol the sources name, exported ----------

(defun symbol-char-p (c)
  (or (alphanumericp c) (find c "-*+!?<>=/%&.")))

(defun export-named (text prefix package)
  "Export from PACKAGE every symbol TEXT writes as PREFIX:NAME."
  (loop for start = (search prefix text) then (search prefix text :start2 (1+ start))
        while start
        do (let* ((from (+ start (length prefix)))
                  (end (or (position-if-not #'symbol-char-p text :start from) (length text))))
             (when (and (> end from)
                        (or (zerop start) (not (symbol-char-p (char text (1- start))))))
               (export (intern (string-upcase (subseq text from end)) package) package)))))

(dolist (name *files*)
  (let ((text (file-text name)))
    (export-named text "nlk:" "NODECODE.KERNEL")
    (export-named text "nle:" "NODECODE.EVOLVED")))

;; What the pure functions call of nodecode, as the release defines it.
(defun nlk::split-words (text)
  (let ((out '()) (word '()))
    (loop for c across (concatenate 'string (or text "") " ")
          do (if (find c '(#\Space #\Tab #\Return #\Newline))
                 (when word (push (coerce (nreverse word) 'string) out) (setf word '()))
                 (push c word)))
    (nreverse out)))
(defvar nlk::*scribe-session-id* nil)
(defmacro nlk::when-let ((variable form) &body body)
  `(let ((,variable ,form)) (when ,variable ,@body)))
(require :sb-md5)
(defun nlk::sha256-text (text)
  ;; A stand-in digest of the release's shape, sha256: and 64 hex digits: the
  ;; tests check the board name's form and determinism, not SHA-256 itself.
  (let ((hex (format nil "~(~{~2,'0x~}~)" (coerce (sb-md5:md5sum-string text :external-format :utf-8) 'list))))
    (concatenate 'string "sha256:" hex hex)))
(defun nlk::trimmed (text) (string-trim '(#\Space #\Tab #\Return #\Newline) text))
(defun nlk::one-line (value &key cap (ellipsis "..."))
  (let ((text (substitute #\Space #\Newline (if (stringp value) value (princ-to-string (or value ""))))))
    (if (and cap (> (length text) cap))
        (concatenate 'string (subseq text 0 (1- cap)) ellipsis)
        text)))

;;; --- read and compile -----------------------------------------------------------

(defparameter *kept* '(defun defparameter defvar defpackage in-package)
  "The forms compiled here; the cell's own macros (define-peripheral,
define-cell) are nodecode's and are only read.")

(defparameter *forms* '())

(dolist (name *files*)
  (let ((*package* (find-package '#:pure-test))
        (count 0))
    (handler-case
        (with-input-from-string (in (file-text name))
          (loop for form = (read in nil :eof)
                until (eq form :eof)
                do (incf count)
                   (when (and (consp form) (eq (first form) 'in-package))
                     (setf *package* (find-package (second form))))
                   (when (and (consp form) (eq (first form) 'defpackage))
                     (eval form))
                   (push (cons *package* form) *forms*)))
      (:no-error (&rest ignore)
        (declare (ignore ignore))
        (check (format nil "~a.lisp reads (~d forms)" name count) t))
      (error (condition)
        (check (format nil "~a.lisp reads" name) nil
               (substitute #\Space #\Newline (princ-to-string condition)))))))
(setf *forms* (nreverse *forms*))

(in-package #:nodecode-meshloop)

;; What define-peripheral gives the cell: the condition and FAIL.
(define-condition meshloop-error (error)
  ((detail :initarg :detail :reader meshloop-error-detail))
  (:report (lambda (c s) (write-string (meshloop-error-detail c) s))))
(defun fail (control &rest arguments)
  (error 'meshloop-error :detail (apply #'format nil control arguments)))
(defun setting (key) (error "no settings outside nodecode: ~s" key))

(in-package #:pure-test)

(let ((warnings '()))
  (handler-bind ((style-warning #'muffle-warning)
                 (warning (lambda (w)
                            (push (princ-to-string w) warnings)
                            (muffle-warning w))))
    (loop for (package . form) in *forms*
          when (and (consp form) (member (first form) *kept*)
                    (not (member (first form) '(defpackage in-package))))
            do (let ((*package* package))
                 (handler-case
                     (multiple-value-bind (function warned failed) (compile nil `(lambda () ,form))
                       (declare (ignore warned))
                       (when failed
                         (push (format nil "~s failed to compile" (second form)) warnings))
                       (funcall function))
                   (error (condition)
                     (push (format nil "~s: ~a" (second form) condition) warnings))))))
  (check "every defun, defparameter and defvar compiles without a full warning"
         (null warnings) (format nil "~{~a~^ | ~}" warnings)))

;;; --- the :tool veto's reading ---------------------------------------------------

(defun veto (text) (nodecode-meshloop::gated-in-text text))

(dolist (case '(("shell string" "(sh \"meshloop accept --task 1 --as me\")")
                ("path and flags" "(sh \"cd /r && /opt/bin/meshloop --json integrate --graph g --into main\")")
                ("argv list" "(sb-ext:run-program \"/x/target/debug/meshloop\" (list \"review-plan\" \"--accept\"))")
                ("quoted argv, prefixed verb" "(uiop:run-program '(\"meshloop\" \"meshloop:accept\" \"--task\" \"1\"))")
                ("format directive" "(sh (format nil \"meshloop ~a --task ~d\" \"accept\" 1))")
                ("windows exe" "(sh \"C:/bin/meshloop.exe accept --task 1 --as me\")")
                ("python argv" "subprocess.run([\"meshloop\", \"integrate\", \"--graph\", \"g\"])")
                ("upper case" "(SH \"MESHLOOP ACCEPT --TASK 1 --AS ME\")")
                ("MCP tool name" "(mcp-call \"meshloop_review_plan\" (list))")
                ("slash line" "(post \"/api/gateway/slash\" \"{\\\"line\\\":\\\"/meshloop accept --task 1\\\"}\")")
                ("prefixed slash" "(post \"/meshloop:integrate\")")
                ("the cell's own code" "(meshloop::accept-of \"s\" \"--task 1 --as me\")")
                ("the cell's package by full name" "(nodecode-meshloop::run-slash \"accept\" \"s\")")
                ;; Review round 1 bypasses.
                ("\\u escape" "(sh \"meshloop \\u0061ccept --task 1\")")
                ("%XX escape" "(http-get \"/x?cmd=meshloop%20%61ccept\")")
                ("empty quotes inside the name" "(sh \"m\\\"\\\"eshloop accept --task 1\")")
                ("single quotes inside the name" "(sh \"mesh''loop accept --task 1\")")
                ("backslash inside the name" "(sh \"me\\\\shloop accept --task 1\")")
                ("verb piped into xargs" "(sh \"echo accept | xargs meshloop\")")
                ("executable in a variable" "(sh \"B=meshloop; $B accept --task 1\")")
                ("quoted variable" "(sh \"x=meshloop && \\\"$x\\\" integrate --graph g\")")
                ("cargo run -p" "(sh \"cargo run -q -p meshloop-cli -- review-plan --accept\")")
                ("find-symbol by name" "(funcall (find-symbol \"ACCEPT-OF\" \"MESHLOOP\") \"s\" \"\")")
                ("intern in the cell's package" "(funcall (intern \"RUN-SLASH\" :nodecode-meshloop) \"accept\" \"s\")")
                ("in-package" "(in-package #:meshloop) (run-slash \"accept --task 1\" \"s\")")
                ;; Review round 2: a word spliced from quoted pieces holds the command.
                ("sh -c with a spliced verb" "sh -c 'meshloop acce''pt --task 1'")
                ("eval of spliced pieces" "eval \"mesh\"\"loop acc\"\"ept --task 1\"")
                ("a wrapper's command" "(sh \"sudo -u me timeout 5 env A=1 meshloop integrate --graph g\")")))
  (check (format nil "veto refuses: ~a" (first case)) (veto (second case)) (second case)))

(dolist (case '(("a read verb" "(sh \"meshloop status --json\")")
                ("the eval verbs" "(list (meshloop:status) (meshloop:report))")
                ("the words in a message" "(sh \"git commit -m 'accept and integrate the meshloop docs'\")")
                ("another program's accept" "(sh \"gh pr review --approve; accept-terms\")")
                ("meshloop doctor then git" "(sh \"meshloop doctor && git status\")")
                ("a status line, then a commit" "(sh \"meshloop status; git commit -m accept\")")
                ("find-package of another package" "(sh \"meshloop status\") (find-package :uiop)")
                ;; Review round 2: a folder named meshloop is no command.
                ("git -C a folder named meshloop" "git -C /home/sam/code/meshloop log --grep accept")
                ("git -C, in a form" "(sh \"git -C /home/sam/code/meshloop log --grep accept\")")
                ("ls a folder named meshloop" "ls ~/code/meshloop | grep integrate")
                ("ls, in a form" "(sh \"ls ~/code/meshloop | grep integrate\")")
                ;; A known gap, kept here so it is not mistaken for coverage: a
                ;; command assembled from pieces at run time. The veto reads text.
                ("KNOWN GAP: a command assembled at run time" "(sh (format nil \"meshloop ~a --task 1\" \"accept\"))")))
  (check (format nil "veto lets through: ~a" (first case)) (null (veto (second case)))
         (veto (second case))))

(let ((args (make-hash-table :test #'equal))
      (strings (make-hash-table :test #'equal)))
  (setf (gethash "form" args) "(sh ($ \"cmd\"))"
        (gethash "cmd" strings) "meshloop accept --task 1 --as me"
        (gethash "strings" args) strings)
  (check "veto reads the strings object of an eval call"
         (nodecode-meshloop::gated-call "eval" args))
  (check "veto refuses an MCP tool by its name"
         (nodecode-meshloop::gated-call "mcp__meshloop__meshloop_accept" (make-hash-table)))
  (check "veto refuses an MCP tool by its last __ segment on the meshloop server"
         (nodecode-meshloop::gated-call "mcp__meshloop__review_plan" (make-hash-table)))
  (check "veto lets another server's accept tool through"
         (null (nodecode-meshloop::gated-call "mcp__github__accept" (make-hash-table))))
  (check "veto lets an unrelated tool through"
         (null (nodecode-meshloop::gated-call "read_file" (make-hash-table)))))

;;; --- the argument checks of the commands that write -----------------------------

(defmacro refuses (form)
  `(handler-case (progn ,form nil)
     (nodecode-meshloop::meshloop-error (c) (nodecode-meshloop::meshloop-error-detail c))))

(in-package #:nodecode-meshloop)

(defun found (text spec) (nth-value 0 (parse-flags text spec)))
(defparameter *accept-spec* '(("--task" :value) ("--as" :value) ("--graph" :value)))

(pure-test::check "identity: typed --as is passed as typed"
                  (equal "claude-coordinator"
                         (identity-of (found "--task 1 --as claude-coordinator" *accept-spec*) "u")))
(pure-test::check "identity: an e-mail-like identity is one"
                  (equal "sam@example.org" (identity-of (found "--as sam@example.org" *accept-spec*) "u")))
(dolist (case '(("no --as" "--task 1")
                ("--as with no value" "--task 1 --as")
                ("a flag as the identity" "--as --graph")
                ("a placeholder" "--as <you>")
                ("the hint's placeholder" "--as <your-name>")
                ("you" "--as you")
                ("ME" "--as ME")
                ("Identity" "--as Identity")
                ("name" "--as name")
                ("non-ASCII" "--as sámuel")
                ("too long" "--as aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")))
  (pure-test::check (format nil "identity refused: ~a" (first case))
                    (pure-test::refuses (identity-of (found (second case) *accept-spec*) "u"))))
(pure-test::check "parse-flags refuses an unknown flag"
                  (pure-test::refuses (found "--task 1 --yes" *accept-spec*)))
(pure-test::check "parse-flags refuses a repeated flag"
                  (pure-test::refuses (found "--as a --as b" *accept-spec*)))
(pure-test::check "parse-flags: --reason takes the rest of the line"
                  (equal "too big, split it"
                         (flag "--reason" (found "--adjust --as me --reason too big, split it"
                                                 '(("--adjust" :switch) ("--as" :value) ("--reason" :rest))))))
(pure-test::check "task: 7 is a task" (equal "7" (task-of (found "--task 007" *accept-spec*) "u")))
(dolist (task '("0" "-1" "x" "1234567890"))
  (pure-test::check (format nil "task refused: ~a" task)
                    (pure-test::refuses (task-of (list (cons "--task" task)) "u"))))
(pure-test::check "graph: g136-live is a graph id" (equal "g136-live" (graph-of "g136-live")))
(pure-test::check "graph refused: --all" (pure-test::refuses (graph-of "--all")))
(pure-test::check "branch: main and feature/x are branches"
                  (and (equal "main" (ref-of "main" "u")) (equal "feature/x" (ref-of "feature/x" "u"))))
(dolist (ref '("" "-f" "a..b" "x.lock" "x/"))
  (pure-test::check (format nil "branch refused: ~s" ref) (pure-test::refuses (ref-of ref "u"))))
(pure-test::check "plan path: the default is meshloop-plan.json"
                  (equal "meshloop-plan.json" (plan-path-of '())))
(pure-test::check "plan path refused: a flag" (pure-test::refuses (plan-path-of '(("--plan" . "--x")))))

;;; --- the Board projection (#137) -----------------------------------------------

(let ((name (board-name-of "/home/op/code/repo")))
  (pure-test::check "board name: meshloop- and 12 hex digits"
                    (and (= (length name) 21) (string= "meshloop-" name :end2 9)
                         (every (lambda (c) (digit-char-p c 16)) (subseq name 9))
                         (every (lambda (c) (or (lower-case-p c) (digit-char-p c) (char= c #\-))) name))
                    name)
  (pure-test::check "board name: the same repository names the same board, with or without a trailing /"
                    (and (equal name (board-name-of "/home/op/code/repo"))
                         (equal name (board-name-of "/home/op/code/repo/"))))
  (pure-test::check "board name: another repository names another board"
                    (not (equal name (board-name-of "/home/op/code/other")))))

(defparameter *evidence* "/r/.meshloop/worktrees/g137/task-1-attempt-2")

(pure-test::check "event post: the fixed template, no mention"
                  (equal (event-post "g137" 17 1 "AwaitingReview" "Verifying" "DeterministicChecksPassed"
                                     ".meshloop/worktrees/g137/task-1-attempt-2" nil 1000)
                         "meshloop g137 event 17: node #1 AwaitingReview (from Verifying, DeterministicChecksPassed); evidence .meshloop/worktrees/g137/task-1-attempt-2")
                  (event-post "g137" 17 1 "AwaitingReview" "Verifying" "DeterministicChecksPassed"
                              ".meshloop/worktrees/g137/task-1-attempt-2" nil 1000))
(pure-test::check "event post: no evidence says none"
                  (equal (event-post "g137" 1 1 "Ready" "Pending" "DependencySatisfied" nil nil 1000)
                         "meshloop g137 event 1: node #1 Ready (from Pending, DependencySatisfied); evidence none"))
(pure-test::check "event post: a mention is the last word, @ and the session id"
                  (let ((text (event-post "g137" 17 1 "AwaitingReview" "Verifying" "DeterministicChecksPassed" nil "s-SUPER" 1000)))
                    (and (suffix-p "; evidence none @s-SUPER" text) (= 1 (count #\@ text)))))
(pure-test::check "event post: two events differ in their text, so the Board's same-words refusal never drops one"
                  (not (equal (event-post "g" 3 1 "Running" "Ready" "AttemptStarted" nil nil 1000)
                              (event-post "g" 9 1 "Running" "Ready" "AttemptStarted" nil nil 1000))))
(pure-test::check "event post: no field carries a mention or free text"
                  (let ((text (event-post "g" 2 1 "accept @s-X now" "/meshloop accept --as x" "Human Acceptance"
                                          "/r/@s-X/wt" nil 1000)))
                    (and (not (find #\@ text)) (not (search "accept" text))
                         (search "node #1 ? (from ?, ?)" text) (search "/r/%40s-X/wt" text)))
                  (event-post "g" 2 1 "accept @s-X now" "/meshloop accept --as x" "Human Acceptance" "/r/@s-X/wt" nil 1000))
(dolist (limit '(1000 160 90 40))
  (let ((text (event-post "graph-with-a-long-id" 123456 12 "AwaitingReview" "Verifying" "DeterministicChecksPassed"
                          (format nil "~{~a~}" (loop repeat 300 collect "x/")) "s-SUPER" limit)))
    (pure-test::check (format nil "event post fits post_chars ~d" limit) (<= (length text) limit) text)))
(let ((text (event-post "g" 5 1 "Blocked" "Pending" "DependencyFailedOrScopeRevoked"
                        (concatenate 'string "/very" (make-string 400 :initial-element #\a) "/task-1-attempt-1") "s-SUPER" 200)))
  (pure-test::check "event post: a long evidence path is shortened from the left, the mention kept"
                    (and (<= (length text) 200) (search "...a" text) (search "/task-1-attempt-1 @s-SUPER" text))
                    text))
(pure-test::check "plan post: the template"
                  (equal (plan-post "g137" "AwaitingPlanReview" 2 nil 1000) "meshloop g137 plan 2: AwaitingPlanReview; evidence none"))
(pure-test::check "evidence: a path under the repository root is relative to it"
                  (and (equal ".meshloop/worktrees/g/t" (relative-to "/r/.meshloop/worktrees/g/t" "/r"))
                       (equal ".meshloop/worktrees/g/t" (relative-to "/r/.meshloop/worktrees/g/t" "/r/"))
                       (equal ".meshloop/worktrees/g/t" (relative-to "C:\\r\\.meshloop\\worktrees\\g\\t" "C:\\r"))))
(pure-test::check "evidence: a path outside the repository shows its last two components only"
                  (and (equal "outside-repo/wt/task-1" (relative-to "/home/op/elsewhere/wt/task-1" "/r"))
                       (equal "outside-repo/wt/task-1" (relative-to "D:\\x\\wt\\task-1\\" "C:\\r"))
                       (equal "outside-repo/t" (relative-to "/t" "/r"))))
(dolist (limit '(60 45 30 12))
  (let ((text (event-post "graph-with-a-long-id" 123456 12 "AwaitingReview" "Verifying" "DeterministicChecksPassed"
                          "/x/y" "s-SUPER" limit)))
    (pure-test::check (format nil "event post: the mention is never cut (post_chars ~d)" limit)
                      (and (<= (length text) limit)
                           (or (suffix-p " @s-SUPER" text) (not (find #\@ text))))
                      text)))
(pure-test::check "event post: with room, a clipped head keeps the whole mention"
                  (suffix-p " @s-SUPER" (event-post "graph-with-a-long-id" 1 1 "Blocked" "Pending" "DependencyFailedOrScopeRevoked" nil "s-SUPER" 30)))

(pure-test::check "mention: wake_on_gates off mentions nobody, on every gate"
                  (every (lambda (state) (null (mention-of state t nil "s-SUPER"))) +gate-states+))
(pure-test::check "mention: on, a current gate names the supervisor"
                  (every (lambda (state) (equal "s-SUPER" (mention-of state t t "s-SUPER"))) +gate-states+))
(pure-test::check "mention: on, a state that is no gate names nobody"
                  (notany (lambda (state) (mention-of state t t "s-SUPER"))
                          '("Pending" "Ready" "Running" "Verifying" "Accepted" "Integrated" "Cancelled" "PlanAccepted")))
(pure-test::check "mention: on, a gate the node has left (history) names nobody"
                  (null (mention-of "AwaitingReview" nil t "s-SUPER")))
(pure-test::check "mention: on, an empty or malformed supervisor names nobody"
                  (notany (lambda (who) (mention-of "Blocked" t t who)) '("" "s X" "@s-X" "s-X;rm" nil)))

(let* ((entries '(("g2" 40 "PlanAccepted" 1 "0123456789abcdef" nil) ("g1" 7 nil 0 nil "e9")))
       (text (cursor-text "meshloop-abc" "/r" entries '("s-OPS" "s-B"))))
  (pure-test::check "cursor: written and read back, with the joined sessions"
                    (multiple-value-bind (read sessions) (parse-cursor text)
                      (and (equal (sort read #'string< :key #'first)
                                  '(("g1" 7 nil 0 nil "e9") ("g2" 40 "PlanAccepted" 1 "0123456789abcdef" nil)))
                           (equal (sort sessions #'string<) '("s-B" "s-OPS"))))
                    text)
  (pure-test::check "cursor: a graph with none starts at 0"
                    (equal (cursor-entry (parse-cursor text) "g9") '("g9" 0 nil 0 nil nil)))
  (pure-test::check "cursor: moving one graph keeps the others"
                    (equal (sort (with-entry (parse-cursor text) (entry-with (cursor-entry (parse-cursor text) "g1") :seq 9 :pending nil))
                                 #'string< :key #'first)
                           '(("g1" 9 nil 0 nil nil) ("g2" 40 "PlanAccepted" 1 "0123456789abcdef" nil)))))
(pure-test::check "cursor: a format-1 file reads, with no ledger, pending or session"
                  (multiple-value-bind (read sessions)
                      (parse-cursor (format nil "nodecode-meshloop board cursor 1~%board b~%repository /r~%graph g1 7 PlanAccepted 1~%"))
                    (and (equal read '(("g1" 7 "PlanAccepted" 1 nil nil))) (null sessions))))
(pure-test::check "cursor: a file this cell did not write is refused, not read as empty"
                  (pure-test::refuses (parse-cursor "something else")))
(pure-test::check "cursor: a plan post in flight names its plan state"
                  (and (pending-word-p "p2:AwaitingPlanReview") (pending-word-p "e17")
                       (not (pending-word-p "p2")) (not (pending-word-p "e1:x")) (not (pending-word-p "p2:a b"))
                       (equal (parse-cursor (format nil "~a~%graph g1 3 PlanAccepted 1 - p2:AwaitingPlanReview~%" +board-cursor-header+))
                              '(("g1" 3 "PlanAccepted" 1 nil "p2:AwaitingPlanReview")))))
(dolist (line '("graph g1 x - 0 - -" "graph g1 1 - 0 ledger! -" "graph g1 1 - 0 - x9" "graph g1 1 - 0 - p2" "session @s-X" "graph --all 1 - 0 - -"))
  (pure-test::check (format nil "cursor: a broken line is refused: ~a" line)
                    (pure-test::refuses (parse-cursor (format nil "~a~%~a~%" +board-cursor-header+ line)))))
(pure-test::check "cursor: kept on another ledger, or past its newest event, starts over"
                  (and (stale-ledger-p '("g" 5 nil 0 "aaaa" nil) "bbbb" 100)
                       (stale-ledger-p '("g" 500 nil 0 "aaaa" nil) "aaaa" 100)
                       (stale-ledger-p '("g" 500 nil 0 nil nil) "aaaa" 100)
                       (not (stale-ledger-p '("g" 5 nil 0 "aaaa" nil) "aaaa" 100))
                       (not (stale-ledger-p '("g" 5 nil 0 nil nil) "aaaa" 100))))
(let ((nodes (make-hash-table :test #'equal)) (latest (make-hash-table :test #'equal)))
  (setf (gethash "1" nodes) "Cancelled" (gethash "1" latest) 7
        (gethash "2" nodes) "AwaitingReview" (gethash "2" latest) 9)
  (pure-test::check "current: the newest counted row reaching the folded state"
                    (current-p 9 2 "AwaitingReview" nodes latest))
  (pure-test::check "current: an older row is history"
                    (not (current-p 8 2 "AwaitingReview" nodes latest)))
  (pure-test::check "current: a row whose state the fold does not show is not current (pre-#167 stale row)"
                    (not (current-p 10 1 "Failed" nodes latest))))

(pure-test::check "typed graphs: --graph, and a status, resume or cancel positional"
                  (and (equal '("g1") (typed-graphs "accept" '("--task" "1" "--as" "p" "--graph" "g1")))
                       (equal '("g2") (typed-graphs "status" '("g2")))
                       (equal '("g3") (typed-graphs "cancel" '("--task" "1" "g3")))
                       (equal '() (typed-graphs "cancel" '("--task" "1")))
                       (equal '() (typed-graphs "run" '("--plan" "plan.json")))))

;;; One-way, read from the sources: the cell calls the Board's join and post
;;; only, never a function that reads posts, holds no :round hook, and the
;;; projection asks meshloop only for the read verbs events and status.
(let* ((board (pure-test::file-text "board"))
       (all (format nil "~{~a~}" (mapcar #'pure-test::file-text pure-test::*files*))))
  (pure-test::check "one-way: no source names the Board's post facts, its delivery or its readers"
                    (notany (lambda (word) (search word all :test #'char-equal))
                            '("board.posted" ":round" "\"POSTS\"" "\"RECENT\"" "\"DELIVER\"" "\"CURSOR\"" "\"DUPLICATE-AT\""
                              "nodecode-board::" "board::")))
  (pure-test::check "one-way: the Board verbs called are JOIN and POST"
                    (loop for start = (search "(board-call " board) then (search "(board-call " board :start2 (1+ start))
                          while start
                          always (let ((name (subseq board (+ start 11) (min (length board) (+ start 60)))))
                                   (or (search "\"JOIN\"" name) (search "\"POST\"" name)
                                       (search "name &rest" name)))))
  (pure-test::check "one-way: the projection runs only meshloop's read verbs events and status"
                    (and (not (search "(act " board))
                         (loop for start = (search "(ask " board) then (search "(ask " board :start2 (1+ start))
                               while start
                               always (let ((verb (subseq board (+ start 5) (+ start 14))))
                                        (or (search "\"events\"" verb) (search "\"status\"" verb)))))))

;;; The pinned nodecode release's board cell defines what this cell calls by name
;;; (JOIN, POST, MEMBER-BOARD of one session, SETTING). Read from its source when
;;; that release is installed here; nothing is loaded or run.
(let ((source (merge-pathnames ".local/share/nodecode/98dd6dc80/src/cells/board/cell.lisp"
                                (user-homedir-pathname))))
  (if (probe-file source)
      (let ((text (pure-test::file-text-at source)))
        (pure-test::check "pinned nodecode 98dd6dc80: the board cell defines member-board (session), join, post and setting"
                          (and (search "(defun member-board (session)" text)
                               (search "(define-verb join (name)" text)
                               (search "(define-verb post (text)" text)
                               (search "(\"post_chars\" :integer" text))))
      (format t "[note] pinned nodecode 98dd6dc80 not installed here: the board cell's source was not checked~%")))

(in-package #:pure-test)
(format t "~d of ~d checks passed~%" (- *checks* *failures*) *checks*)
(sb-ext:exit :code (if (zerop *failures*) 0 1))
