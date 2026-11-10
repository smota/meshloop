;;;; drive.lisp --- the commands that write, /meshloop, and the cell's declaration.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; plan, review-plan, run, resume, cancel, accept and integrate are thin calls
;;;; to the meshloop binary, made only from the /meshloop slash command the
;;;; operator types. None is an eval verb or a tool, none is called from a hook,
;;;; a timer or a Board post, and nothing here follows one with another: run
;;;; starts a detached driver and says so; accept, resume and integrate wait for
;;;; the operator. With meshloop.write_commands false (the default) each one
;;;; refuses in one line before it runs anything.
;;;;
;;;; Human gates stay meshloop's (ADR 0009, ADR 0038). review-plan without a
;;;; decision only shows the plan; the decision is a second command the person
;;;; types, naming themselves with --as. accept takes the identity typed in that
;;;; command and passes it as --as: there is no default and no setting for it.
;;;; integrate lands only with --accept-integrate typed, after the person has had
;;;; the integrate worktree pointed out to them.

(in-package #:nodecode-meshloop)

;;; --- the words of a command -------------------------------------------------------

(defun parse-flags (text spec)
  "TEXT's words read against SPEC, a list of (FLAG KIND): KIND :value takes the
next word, :switch none, :rest every word after it as one text.
=> (values FOUND POSITIONALS), FOUND an alist of FLAG to its value (T for a
switch). An unknown flag, a repeated one or a missing value is a refusal."
  (let ((words (nlk:split-words (or text ""))) (found '()) (positionals '()))
    (loop while words
          do (let* ((word (pop words))
                    (entry (assoc word spec :test #'string=)))
               (cond ((null entry)
                      (if (and (> (length word) 1) (char= #\- (char word 0)))
                          (fail "takes no ~a; it takes ~{~a~^, ~}"
                                (nlk:one-line word :cap 40) (mapcar #'first spec))
                          (push word positionals)))
                     ((assoc word found :test #'string=)
                      (fail "~a is given twice" word))
                     (t (ecase (second entry)
                          (:switch (push (cons word t) found))
                          (:value (let ((value (pop words)))
                                    (unless value (fail "~a needs a value" word))
                                    (push (cons word value) found)))
                          (:rest (unless words (fail "~a needs text after it" word))
                                 (push (cons word (format nil "~{~a~^ ~}" words)) found)
                                 (setf words '())))))))
    (values found (nreverse positionals))))

(defun flag (name found)
  "The value FOUND holds for flag NAME, or NIL."
  (cdr (assoc name found :test #'string=)))

(defun no-positionals (positionals usage)
  "Refuse stray words, naming USAGE."
  (when positionals
    (fail "~s is not part of it; type ~a" (nlk:one-line (first positionals) :cap 40) usage)))

(defun ascii-word-p (text allowed &key (max 128))
  "Whether TEXT is 1 to MAX of ASCII letters, digits and ALLOWED, not starting
with - so it cannot read as a flag."
  (and (stringp text) (<= 1 (length text) max)
       (char/= #\- (char text 0))
       (every (lambda (c) (or (and (< (char-code c) 128) (alphanumericp c)) (find c allowed))) text)))

(defparameter +placeholder-identities+ '("you" "me" "identity" "name" "your-name" "yourname")
  "Words a hint or a habit puts after --as that name nobody.")

(defun identity-of (found usage)
  "The identity typed after --as in this command, checked; a refusal when there
is none. The cell supplies no identity of its own: not a default, not a
setting, not the session's."
  (let ((who (flag "--as" found)))
    (cond ((null who)
           (fail "names no identity. Type who decides after --as, in this command: ~a" usage))
          ((or (member who +placeholder-identities+ :test #'string-equal)
               (and (> (length who) 1) (char= #\< (char who 0))))
           (fail "~s is a placeholder, not a name: type your own name after --as"
                 (nlk:one-line who :cap 40)))
          ((not (ascii-word-p who "._@:+-"))
           (fail "~s is no identity: letters, digits and . _ @ : + -, at most 128, not starting with -"
                 (nlk:one-line who :cap 40)))
          (t who))))

(defun task-of (found usage)
  "The task id typed after --task, a positive integer."
  (let ((task (flag "--task" found)))
    (unless (and task (<= 1 (length task) 9) (every #'digit-char-p task)
                 (plusp (parse-integer task)))
      (fail "needs --task N, a node id: ~a" usage))
    (princ-to-string (parse-integer task))))

(defun graph-of (text)
  "TEXT as a graph id, or a refusal."
  (unless (graph-id-p text)
    (fail "~s is no graph id: letters, digits and . _ : -, at most 128, not starting with -"
          (nlk:one-line text :cap 40)))
  text)

(defun ref-of (text usage)
  "TEXT as a branch to land on, or a refusal; git checks the rest."
  (unless (and (ascii-word-p text "/._-" :max 200)
               (not (search ".." text))
               (not (suffix-p "/" text)) (not (suffix-p ".lock" text)))
    (fail "needs --into BRANCH, a branch name: ~a" usage))
  text)

(defun plan-path-of (found)
  "The plan file after --plan, meshloop's default meshloop-plan.json without one."
  (let ((path (or (flag "--plan" found) (flag "--out" found) "meshloop-plan.json")))
    (unless (and (<= 1 (length path) 1024)
                 (char/= #\- (char path 0))
                 (notany (lambda (c) (< (char-code c) 32)) path))
      (fail "~s is no plan path" (nlk:one-line path :cap 40)))
    path))

;;; --- the write switch -------------------------------------------------------------

(defun writes-on (verb)
  "Refuse VERB in one line unless meshloop.write_commands is on."
  (unless (setting :write-commands)
    (fail "refused: write commands are off (meshloop.write_commands is false, the default). ~
           To let this cell ~a, set it to true in the nodecode home's config.jsonc and restart ~
           the cell; `meshloop ~a' in a shell is unaffected" verb verb)))

;;; --- reads the write commands lean on ---------------------------------------------

(defun ok-or-refuse (object)
  "OBJECT when its ok is true, else the refusal it carries."
  (if (nlk:json-value object :boolean "ok") object (fail "~a" (refusal-of object))))

(defun newest-graph (session-id)
  "The newest run's graph id in SESSION-ID's repository, or a refusal."
  (let* ((object (ok-or-refuse (ask "status" '() session-id)))
         (data (nlk:json-value object :object "data")))
    (unless (and data (nth-value 1 (gethash "graph_id" data)))
      (checked object "status_empty")
      (fail "names no graph and this repository has no managed run"))
    (nlk:json-value (checked object "status") :string "data" "graph_id")))

(defun graph-or-newest (positionals session-id usage)
  "The one graph id among POSITIONALS, else the newest run's."
  (cond ((rest positionals) (fail "takes one graph id at most: ~a" usage))
        (positionals (graph-of (first positionals)))
        (t (newest-graph session-id))))

(defun run-status (graph session-id)
  "Status of GRAPH, checked."
  (checked (ok-or-refuse (ask "status" (list "--graph" graph) session-id)) "status"))

(defun settled-status (graph session-id &key (tries 10))
  "GRAPH's status data once its driver is no longer alive, or after TRIES
half-second waits: a cancelled driver takes a moment to see the cancel."
  (loop for try from 1
        for data = (nlk:json-value (run-status graph session-id) :object "data")
        when (or (>= try tries) (not (equal (nlk:json-value data :string "driver") "alive")))
          return data
        do (sleep 0.5)))

(defun driver-alive-p (graph session-id)
  "Whether meshloop reports GRAPH's driver alive now."
  (equal (nlk:json-value (run-status graph session-id) :string "data" "driver") "alive"))

(defun graph-lines (graph)
  "One line per node of a plan GRAPH: id, tier, what it waits for, description."
  (let ((nodes (nlk:json-array graph "nodes")))
    (with-output-to-string (out)
      (loop for node across nodes
            for index from 0
            while (< index *node-lines*)
            do (let ((after (coerce (nlk:json-array node "depends_on") 'list)))
                 (format out "~%  #~a~@[ ~a~]~:[~*~;, after ~{#~a~^, ~}~]: ~a"
                         (nlk:json-value node :integer "id")
                         (nlk:json-value node :string "tier")
                         after after
                         (or (clipped (nlk:json-value node :any "description")) "(no description)"))))
      (when (> (length nodes) *node-lines*)
        (format out "~%  ... ~d more in the plan file" (- (length nodes) *node-lines*))))))

(defun checked-graph (graph)
  "GRAPH, a plan meshloop answered, once the contract's graph fields are in it."
  (nlk:when-let (missing (missing-field graph (nlk:json-array *contract* "graph")))
    (older-binary (format nil "data.graph.~a" missing)))
  graph)

(defparameter *plan-limit* (* 1024 1024)
  "The largest plan file review-plan reads to show.")

(defun read-plan (session-id path)
  "The plan file PATH, relative to SESSION-ID's repository root, decoded and
checked against the contract's graph fields."
  (let* ((root (uiop:ensure-directory-pathname (session-directory session-id)))
         (file (uiop:merge-pathnames* (uiop:parse-native-namestring path) root)))
    (unless (uiop:file-exists-p file)
      (fail "no plan at ~a; /meshloop plan OBJECTIVE writes one" (uiop:native-namestring file)))
    (when (> (with-open-file (in file :element-type '(unsigned-byte 8)) (file-length in)) *plan-limit*)
      (fail "~a is larger than ~d bytes; that is no plan to show" path *plan-limit*))
    (let ((graph (handler-case (nlk:decode-json (uiop:read-file-string file :external-format :utf-8)
                                                :whole t)
                   (error () (fail "~a is not JSON" path)))))
      (nlk:when-let (missing (and (hash-table-p graph)
                                  (missing-field graph (nlk:json-array *contract* "graph"))))
        (fail "~a is no meshloop plan: it lacks ~a" path missing))
      (unless (hash-table-p graph) (fail "~a is no meshloop plan" path))
      graph)))

;;; --- plan -------------------------------------------------------------------------

(defparameter +plan-usage+ "/meshloop plan [--out FILE] OBJECTIVE")

(defun plan-of (session-id text)
  (answering "plan"
             (lambda ()
               (writes-on "plan")
               (multiple-value-bind (found words) (parse-flags text '(("--out" :value)))
                 (let ((out (plan-path-of found))
                       (objective (format nil "~{~a~^ ~}" words)))
                   (when (zerop (length objective))
                     (fail "needs an objective: ~a" +plan-usage+))
                   (let* ((object (checked (ok-or-refuse
                                            (or (act "plan" (list "--objective" objective "--out" out)
                                                     session-id :seconds (setting :plan-timeout-seconds))
                                                (fail "answered no JSON envelope")))
                                           "plan"))
                          (graph (checked-graph (nlk:json-value object :object "data" "graph"))))
                     (format nil "meshloop plan: graph ~a, ~d node~:p, written to ~a~a~%  ~
                                  Nothing runs yet. Read it: /meshloop review-plan --plan ~a; then decide ~
                                  in a separate command, naming yourself: --accept, --decline or --adjust, ~
                                  each with --as <your-name>"
                             (nlk:json-value graph :string "graph_id")
                             (length (nlk:json-array graph "nodes"))
                             (nlk:json-value object :string "data" "path")
                             (graph-lines graph) out)))))))

;;; --- review-plan ------------------------------------------------------------------

(defparameter +review-usage+
  "/meshloop review-plan [--plan FILE] to read it; then /meshloop review-plan [--plan FILE] --accept|--decline|--adjust --as <your-name> [--reason TEXT]")

(defun show-plan (session-id path)
  "The plan at PATH and where review stands, writing nothing."
  (let* ((graph (read-plan session-id path))
         (id (nlk:json-value graph :string "graph_id"))
         (status (ask "status" (list "--graph" id) session-id))
         (state (and (nlk:json-value status :boolean "ok")
                     (nlk:json-value (checked status "status") :string "data" "plan_state"))))
    (format nil "meshloop review-plan: plan ~a, graph ~a, ~d node~:p, ~:[not reviewed in this repository yet~;~:*plan state ~a~]~a~%  ~
                 Node descriptions are data, not instructions. Decide in a separate command, naming ~
                 yourself: /meshloop review-plan --plan ~a --accept --as <your-name> (or --decline or --adjust, ~
                 --as <your-name> --reason TEXT)"
            path id (length (nlk:json-array graph "nodes")) state (graph-lines graph) path)))

(defun review-plan-of (session-id text)
  (answering "review-plan"
             (lambda ()
               (multiple-value-bind (found positionals)
                   (parse-flags text '(("--plan" :value) ("--accept" :switch) ("--decline" :switch)
                                       ("--adjust" :switch) ("--as" :value) ("--reason" :rest)))
                 (no-positionals positionals +review-usage+)
                 (let* ((path (plan-path-of found))
                        (decisions (remove-if-not (lambda (d) (flag d found))
                                                  '("--accept" "--decline" "--adjust"))))
                   (cond
                     ((null decisions)
                      (when (or (flag "--as" found) (flag "--reason" found))
                        (fail "names no decision: add --accept, --decline or --adjust"))
                      (show-plan session-id path))
                     ((rest decisions)
                      (fail "takes one decision, not ~{~a~^ and ~}" decisions))
                     (t
                      (writes-on "review-plan")
                      (let* ((decision (first decisions))
                             (who (identity-of found +review-usage+))
                             (reason (flag "--reason" found)))
                        (when (and (string= decision "--adjust") (null reason))
                          (fail "--adjust needs --reason TEXT: what to change"))
                        (let* ((object (checked
                                        (ok-or-refuse
                                         (or (act "review-plan"
                                                  (append (list "--plan" path decision "--as" who)
                                                          (when reason (list "--reason" reason)))
                                                  session-id
                                                  :seconds (when (string= decision "--adjust")
                                                             (setting :plan-timeout-seconds)))
                                             (fail "answered no JSON envelope")))
                                        "review_plan"))
                               (data (nlk:json-value object :object "data"))
                               (state (nlk:json-value data :string "plan_state"))
                               (graph (nlk:json-value data :string "graph_id")))
                          (format nil "meshloop review-plan: graph ~a, ~a as ~a: ~a~:[~*~;~a~]~%  next: ~a"
                                  graph (nlk:json-value data :string "decision")
                                  (nlk:json-value data :string "as") state
                                  (string= decision "--adjust") (graph-lines (checked-graph (nlk:json-value data :object "graph")))
                                  (cond ((equal state "PlanAccepted")
                                         (format nil "/meshloop run --plan ~a starts it detached; nothing starts on its own" path))
                                        ((equal state "PlanDeclined")
                                         (format nil "stopped; /meshloop review-plan --plan ~a --adjust --as <your-name> --reason TEXT reopens it" path))
                                        (t (format nil "read it again with /meshloop review-plan --plan ~a, then decide" path)))))))))))))

;;; --- run, resume, cancel ----------------------------------------------------------

(defun driver-answer (verb object)
  "The answer for a detached driver run or resume started."
  (let* ((data (nlk:json-value object :object "data"))
         (graph (nlk:json-value data :string "graph_id")))
    (format nil "meshloop ~a: graph ~a, driver started detached (~a); nothing follows on its own~%  ~
                 follow it with /meshloop inspect ~a; a node that waits for acceptance takes ~
                 /meshloop accept --task N --as <your-name> --graph ~a. While this driver is alive it ~
                 merges accepted nodes itself; once /meshloop status ~a shows it exited, ~
                 /meshloop resume ~a starts the next~%  driver log: ~a"
            verb graph (nlk:json-value data :string "status") graph graph graph graph
            (nlk:json-value data :string "driver_log"))))

(defun run-plan-of (session-id text)
  (answering "run"
             (lambda ()
               (writes-on "run")
               (multiple-value-bind (found positionals) (parse-flags text '(("--plan" :value)))
                 (no-positionals positionals "/meshloop run [--plan FILE]")
                 ;; Never --accept-plan: the plan's acceptance is review-plan's,
                 ;; typed by the person. A plan not accepted is refused by meshloop.
                 (driver-answer "run"
                                (checked (ok-or-refuse
                                          (or (act "run" (list "--plan" (plan-path-of found) "--detach")
                                                   session-id)
                                              (fail "answered no JSON envelope")))
                                         "run_detached"))))))

(defun resume-of (session-id text)
  (answering "resume"
             (lambda ()
               (writes-on "resume")
               (multiple-value-bind (found positionals)
                   (parse-flags text '(("--retry" :switch) ("--restart" :switch)))
                 (let ((graph (graph-or-newest positionals session-id "/meshloop resume [GRAPH] [--retry|--restart]")))
                   (when (driver-alive-p graph session-id)
                     (fail "refused: a driver is running for ~a; it merges accepted nodes itself. ~
                            Follow it with /meshloop inspect ~a" graph graph))
                   (driver-answer "resume"
                                  (checked (ok-or-refuse
                                            (or (act "resume" (append (list "--detach" "--graph" graph)
                                                                      (when (flag "--retry" found) (list "--retry"))
                                                                      (when (flag "--restart" found) (list "--restart")))
                                                     session-id)
                                                (fail "answered no JSON envelope")))
                                           "resume_detached")))))))

(defun cancel-of (session-id text)
  (answering "cancel"
             (lambda ()
               (writes-on "cancel")
               (multiple-value-bind (found positionals) (parse-flags text '(("--task" :value)))
                 (let* ((usage "/meshloop cancel [GRAPH] [--task N]")
                        (graph (graph-or-newest positionals session-id usage))
                        (task (and (flag "--task" found) (task-of found usage))))
                   (checked (ok-or-refuse
                             (or (act "cancel" (append (list "--graph" graph) (when task (list "--task" task)))
                                      session-id)
                                 (fail "answered no JSON envelope")))
                            "cancel")
                   ;; What is left, as meshloop sees it: attempts still alive,
                   ;; the driver, and the worktrees it keeps for the run. The
                   ;; cancel is recorded by now, whatever these reads answer.
                   (handler-case (cancel-aftermath graph task session-id)
                     (meshloop-error (condition)
                       (format nil "meshloop cancel: graph ~a~@[ task ~a~] cancelled; reading what is left ~
                                    failed: ~a" graph task (meshloop-error-detail condition)))))))))

(defun cancel-aftermath (graph task session-id)
  "The answer after GRAPH (or its TASK) was cancelled: what meshloop reports left."
                   (let* ((status (settled-status graph session-id))
                          (nodes (nlk:json-array status "nodes"))
                          (alive (loop for node across nodes
                                       when (equal (nlk:json-value node :string "live") "Live")
                                         collect (nlk:json-value node :integer "id")))
                          (inventory (checked (ok-or-refuse
                                               (ask "workspace" (list "inventory") session-id
                                                    :command "meshloop:workspace-inventory"))
                                              "workspace_inventory"))
                          (entries (remove-if-not (lambda (entry) (equal (nlk:json-value entry :string "graph_id") graph))
                                                  (nlk:json-array inventory "data" "entries")))
                          (foreign (remove "ledger-owned" (map 'list (lambda (entry) (nlk:json-value entry :string "ownership")) entries)
                                           :test #'equal)))
                     (format nil "meshloop cancel: graph ~a~@[ task ~a~] cancelled; ~a~%  ~
                                  attempt processes alive: ~:[none~;~:*~{#~a~^, ~}~]~@[ (driver ~a)~]~%  ~
                                  workspace entries meshloop keeps for ~a (worktrees and their run folder): ~d~:[~*, every one ledger-owned~;, NOT ledger-owned: ~{~a~^, ~}~]; ~
                                  `meshloop workspace preview --cleanup --json' says which it may reclaim"
                             graph task (state-counts nodes) alive
                             (nlk:json-value status :string "driver")
                             graph (length entries) foreign foreign)))

;;; --- accept -----------------------------------------------------------------------

(defparameter +accept-usage+ "/meshloop accept --task N --as <your-name> [--graph GRAPH]")

(defun awaiting-graph (session-id)
  "The one run waiting for acceptance, when accept names no --graph. More than
one is a refusal asking which; none falls back to the newest run."
  (let* ((object (ok-or-refuse (ask "status" '() session-id)))
         (data (nlk:json-value object :object "data")))
    (unless (and data (nth-value 1 (gethash "graph_id" data)))
      (fail "names no graph and this repository has no managed run"))
    (let ((waiting (coerce (nlk:json-array (checked object "status") "data" "awaiting_acceptance") 'list)))
      (cond ((rest waiting)
             (fail "~d runs wait for acceptance (~{~a~^, ~}); name one with --graph" (length waiting) waiting))
            (waiting (first waiting))
            (t (nlk:json-value data :string "graph_id"))))))

(defun accept-of (session-id text)
  (answering "accept"
             (lambda ()
               (writes-on "accept")
               (multiple-value-bind (found positionals)
                   (parse-flags text '(("--task" :value) ("--as" :value) ("--graph" :value)))
                 (no-positionals positionals +accept-usage+)
                 (let* ((task (task-of found +accept-usage+))
                        (who (identity-of found +accept-usage+))
                        (graph (if (flag "--graph" found)
                                   (graph-of (flag "--graph" found))
                                   (awaiting-graph session-id))))
                   ;; WHO is the word typed after --as in this command; nothing
                   ;; else reaches this argument.
                   (act "accept" (list "--task" task "--as" who "--graph" graph) session-id)
                   ;; What meshloop recorded, read back from its event log.
                   (let* ((object (checked (ok-or-refuse (ask "inspect" (list "--attempts" "--graph" graph) session-id))
                                           "inspect_attempts"))
                          (node (find-if (lambda (node) (equal (princ-to-string (nlk:json-value node :integer "id")) task))
                                         (nlk:json-array object "data" "nodes")))
                          (recorded (and node (last-attempt node) (acceptance-line (last-attempt node)))))
                     (format nil "meshloop accept: task ~a of graph ~a accepted as ~a; ~:[meshloop shows no acceptance record for it yet~;~:*meshloop recorded \"~a\"~]~%  ~
                                  next: ~:[/meshloop resume ~a merges it; nothing resumes on its own~;~
                                  the driver running for ~a merges it itself; type nothing more~]"
                             task graph who recorded (driver-alive-p graph session-id) graph)))))))

;;; --- integrate --------------------------------------------------------------------

(defparameter +integrate-usage+ "/meshloop integrate --graph GRAPH --into BRANCH --accept-integrate")

(defun integrate-of (session-id text)
  (answering "integrate"
             (lambda ()
               (writes-on "integrate")
               (multiple-value-bind (found positionals)
                   (parse-flags text '(("--graph" :value) ("--into" :value) ("--accept-integrate" :switch)))
                 (no-positionals positionals +integrate-usage+)
                 (let ((graph (if (flag "--graph" found)
                                  (graph-of (flag "--graph" found))
                                  (fail "names no graph: ~a" +integrate-usage+)))
                       (into (ref-of (or (flag "--into" found) "") +integrate-usage+)))
                   (if (not (flag "--accept-integrate" found))
                       ;; Not landed: point at what the person accepts by typing it.
                       (let* ((object (ok-or-refuse (ask "inspect" (list "--graph" graph) session-id)))
                              (export (nlk:json-value object :object "data" "git_export")))
                         (when export (checked object "integrate_preview"))
                         (format nil "meshloop integrate: graph ~a not landed. ~a, then type ~
                                      /meshloop integrate --graph ~a --into ~a --accept-integrate"
                                 graph
                                 (if export
                                     (format nil "Review the integrate worktree ~a (~a at ~a)"
                                             (nlk:json-value export :string "worktree_path")
                                             (nlk:json-value export :string "branch_ref")
                                             (nlk:json-value export :string "commit_sha"))
                                     "It has no integrate worktree yet")
                                 graph into))
                       (multiple-value-bind (object said)
                           (act "integrate" (list "--graph" graph "--into" into "--accept-integrate") session-id)
                         (when object (ok-or-refuse object))
                         (let ((nodes (nlk:json-array (run-status graph session-id) "data" "nodes"))
                               (lines (remove-if (lambda (line) (zerop (length (nlk:trimmed line))))
                                                 (uiop:split-string (or said "") :separator '(#\Newline)))))
                           (format nil "meshloop integrate: graph ~a landed on ~a; ~a~{~%  meshloop: ~a~}"
                                   graph into (state-counts nodes)
                                   (loop for line in lines for i below 4
                                         collect (nlk:one-line line :cap (* 2 *text-cap*))))))))))))

;;; --- /meshloop --------------------------------------------------------------------

(defun panel ()
  "The dialog bare /meshloop answers: one row a read-only verb."
  (nle:list-dialog
   :title "Meshloop" :summary "This folder's meshloop runs; writes are typed, off by default"
   :rows (list (nle:run-row "Doctor" "/meshloop doctor" :detail "config and harness readiness")
               (nle:run-row "Status" "/meshloop status" :detail "the newest run and its nodes")
               (nle:run-row "Report" "/meshloop report" :detail "harnesses and model_refs dispatched")
               (nle:ask-row "Inspect a run" "Graph" "/meshloop inspect"
                            :placeholder "a graph id, or nothing for the newest" :verb "inspect")
               (nle:ask-row "Read a plan" "Plan" "/meshloop review-plan --plan"
                            :placeholder "meshloop-plan.json" :verb "read"))))

(defparameter +usage+
  "meshloop: /meshloop doctor | status [GRAPH] | inspect [GRAPH] | report [GRAPH] | review-plan [--plan FILE] read; plan, review-plan --accept|--decline|--adjust --as <your-name>, run, resume, cancel, accept --task N --as <your-name>, integrate --graph G --into BRANCH --accept-integrate write, when meshloop.write_commands is on (off by default); hook-check FORM tests the :tool veto")

(defun after-first-word (text)
  "TEXT past its first word and the blanks after it, as typed."
  (let* ((text (nlk:trimmed (or text "")))
         (end (position-if (lambda (c) (find c '(#\Space #\Tab #\Return #\Newline))) text)))
    (if end (nlk:trimmed (subseq text end)) "")))

(defun run-slash (args session-id)
  "/meshloop VERB ...: the operator's command. Typed alone, the panel.
=> the answer's text."
  (let* ((words (nlk:split-words (or args "")))
         (verb (string-downcase (or (first words) "")))
         (rest (format nil "~{~a~^ ~}" (rest words))))
    (cond ((null words) (values +usage+ (panel)))
          ((string= verb "doctor") (doctor-of session-id rest))
          ((string= verb "status") (view-of "status" session-id rest))
          ((string= verb "inspect") (view-of "inspect" session-id rest))
          ((string= verb "report") (report-of session-id rest))
          ((string= verb "hook-check") (hook-check-of (after-first-word args)))
          ((string= verb "plan") (plan-of session-id rest))
          ((string= verb "review-plan") (review-plan-of session-id rest))
          ((string= verb "run") (run-plan-of session-id rest))
          ((string= verb "resume") (resume-of session-id rest))
          ((string= verb "cancel") (cancel-of session-id rest))
          ((string= verb "accept") (accept-of session-id rest))
          ((string= verb "integrate") (integrate-of session-id rest))
          (t (format nil "meshloop: ~s is no verb here; ~a" (nlk:one-line verb :cap 40)
                     (subseq +usage+ (length "meshloop: ")))))))

;;; --- the declaration --------------------------------------------------------------

(nle:define-cell meshloop
  (:section ("meshloop")
    (:guide "meshloop runs through `meshloop VERB --json' in the session's repository. binary is a meshloop built from the checkout. write_commands lets the slash commands that write run; it is off by default")
    ("binary" :string :default "meshloop"
              :doc "the meshloop executable: the absolute path of target/debug/meshloop built from the checkout, or a name found on PATH")
    ("timeout_seconds" :integer :default 60 :min 1
                       :doc "how long one meshloop call may run before it is stopped")
    ("plan_timeout_seconds" :integer :default 600 :min 1
                            :doc "how long plan and review-plan --adjust, which run the planner in the foreground, may run")
    ("write_commands" :boolean :default nil
                      :doc "let /meshloop plan, review-plan with a decision, run, resume, cancel, accept and integrate run; off, each refuses"))
  (:help :meshloop "meshloop:doctor, status, inspect, report read meshloop runs; /meshloop drives them when write_commands is on" +primer+)
  (:command "meshloop" 'run-slash
            :description "Meshloop runs in this folder: read them, and drive them when write_commands is on")
  (:hook :tool #'veto-gated-calls)
  (:start (lambda ()
            (setf *contract* (load-contract))
            (nle:on-stop (lambda () (setf *contract* nil))))))
