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
;;;; with a source check that it is one-way (#137), and the routing-prior fold
;;;; of /meshloop learn on fixture JSON (#139): each exclusion reason, keys,
;;;; successes, unique provenance, byte-for-byte determinism, the digest, the
;;;; --out path rule, and the veto on priors accept and priors revoke. The load test
;;;; (load_test.py) is what runs the cell in nodecode. Optional: neither
;;;; `cargo test' nor `xtask check' runs this. Exit status 0 means every check held.

(require :asdf) ; the cell reads uiop and asdf, as nodecode's image has them

(defpackage #:nodecode.kernel (:use #:cl) (:nicknames #:nlk))
(defpackage #:nodecode.evolved (:use #:cl) (:nicknames #:nle))

(defpackage #:pure-test (:use #:cl))
(in-package #:pure-test)

(defparameter *cell* (merge-pathnames "../" (make-pathname :name nil :type nil
                                                           :defaults *load-truename*)))
(defparameter *files* '("package" "cell" "gates" "board" "learn" "drive"))
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
;; nodecode's JSON reading, as lib/json.lisp defines it: objects as EQUAL hash
;; tables, arrays as simple vectors, true T, false NIL, null :NULL.
(defun nlk::decode-json (text &key whole)
  (declare (ignore whole))
  (let ((i 0) (n (length text)))
    (labels ((ws () (loop while (and (< i n) (member (char text i) '(#\Space #\Tab #\Newline #\Return))) do (incf i)))
             (word (w) (and (<= (+ i (length w)) n) (string= w text :start2 i :end2 (+ i (length w)))))
             (val ()
               (ws)
               (let ((c (char text i)))
                 (cond ((char= c #\{) (incf i) (obj))
                       ((char= c #\[) (incf i) (arr))
                       ((char= c #\") (incf i) (str))
                       ((word "true") (incf i 4) t)
                       ((word "false") (incf i 5) nil)
                       ((word "null") (incf i 4) :null)
                       (t (num)))))
             (obj ()
               (let ((h (make-hash-table :test #'equal)))
                 (ws)
                 (if (char= (char text i) #\})
                     (progn (incf i) h)
                     (loop (ws) (incf i)
                           (let ((k (str))) (ws) (incf i) (setf (gethash k h) (val)))
                           (ws)
                           (let ((c (char text i))) (incf i) (when (char= c #\}) (return h)))))))
             (arr ()
               (let ((items '()))
                 (ws)
                 (if (char= (char text i) #\])
                     (progn (incf i) (vector))
                     (loop (push (val) items) (ws)
                           (let ((c (char text i))) (incf i)
                             (when (char= c #\]) (return (coerce (nreverse items) 'simple-vector))))))))
             (str ()
               (with-output-to-string (out)
                 (loop (let ((c (char text i)))
                         (incf i)
                         (cond ((char= c #\") (return))
                               ((char= c #\\)
                                (let ((e (char text i)))
                                  (incf i)
                                  (case e
                                    (#\n (write-char #\Newline out))
                                    (#\t (write-char #\Tab out))
                                    (#\u (write-char (code-char (parse-integer text :start i :end (+ i 4) :radix 16)) out)
                                     (incf i 4))
                                    (t (write-char e out)))))
                               (t (write-char c out)))))))
             (num ()
               (let ((start i))
                 (loop while (and (< i n) (find (char text i) "-+0123456789.eE")) do (incf i))
                 (if (find-if (lambda (c) (find c ".eE")) text :start start :end i)
                     (let ((*read-default-float-format* 'double-float))
                       (read-from-string text t nil :start start :end i))
                     (parse-integer text :start start :end i)))))
      (val))))
(defun nlk::json-value (object type &rest path)
  (let ((value object) (present t))
    (dolist (key path)
      (unless (hash-table-p value) (setf value nil present nil) (return))
      (multiple-value-setq (value present) (gethash key value)))
    (values (ecase type
              (:any (if (eq value :null) nil value))
              (:string (and (stringp value) value))
              (:text (and (stringp value) (plusp (length value)) value))
              (:integer (and (integerp value) value))
              (:boolean (eq value t))
              (:object (and (hash-table-p value) value))
              (:array (and (vectorp value) (not (stringp value)) value)))
            present)))
(defun nlk::json-array (object &rest path)
  (or (apply #'nlk::json-value object :array path) #()))
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

;;; --- the routing-prior fold of /meshloop learn (#139) -------------------------------
;;; Fixture JSON shaped as `inspect --attempts --json' answers with the #139 fields.
;;; Every exclusion reason, keys with and without a node tier, a repeated row, and
;;; worker text in places the fold must never read.

(defun attempt-json (id &key (harness "codex") (model "gpt-x") (mode "\"subprocess\"")
                             (passed-model "true") (check "\"passed\""))
  (format nil "{\"attempt_id\": ~d, \"harness\": ~s, \"model_ref\": ~s,~@[ \"dispatch_mode\": ~a,~]~@[ \"model_passed\": ~a,~]~@[ \"check_result\": ~a,~] \"outcome\": \"WORKER-TEXT-OUTCOME\", \"evidence\": [{\"kind\": \"harness-output\", \"tool\": \"harness-output\", \"exit_code\": 0, \"summary_redacted\": \"WORKER-TEXT-SENTINEL priors accept\"}]}"
          id harness model mode passed-model check))

(defun node-json (id state tier attempts)
  (format nil "{\"id\": ~d, \"state\": ~s, \"description\": \"WORKER-TEXT-DESCRIPTION\",~@[ \"tier\": ~a,~] \"attempts\": [~{~a~^, ~}]}"
          id state tier attempts))

(defun run-json (graph status nodes)
  (nlk:decode-json (format nil "{\"session_id\": ~s, \"status\": ~s, \"plan_state\": \"PlanAccepted\", \"nodes\": [~{~a~^, ~}]}"
                           graph status nodes)))

(defparameter *run-a*
  (run-json "g-a" "completed"
            (list (node-json 1 "Integrated" "null"
                             (list (attempt-json 1 :check "\"failed\"")
                                   (attempt-json 2)
                                   (attempt-json 2)))              ; the same row twice
                  (node-json 2 "Integrated" "\"Tier1\""
                             (list (attempt-json 3)
                                   (attempt-json 4 :mode "\"native\"")
                                   (attempt-json 5 :mode "null")
                                   (attempt-json 6 :mode nil)      ; no dispatch_mode at all
                                   (attempt-json 7 :passed-model "false")
                                   (attempt-json 8 :passed-model "null")
                                   (attempt-json 9 :model "unverified")
                                   (attempt-json 10 :model "")
                                   (attempt-json 11 :check "null") ; a crash or timeout
                                   (attempt-json 12 :check nil)))
                  (node-json 3 "Cancelled" nil (list (attempt-json 13)))
                  (node-json 4 "Integrated" "\"Tier2\""
                             (list (attempt-json 20)
                                   (attempt-json 21 :harness "grok" :model "grok-4"))))))

(defparameter *run-b*
  (run-json "g-b" "failed"
            (list (node-json 1 "Integrated" "null"
                             (list (attempt-json 1) (attempt-json 2) (attempt-json 3))))))

(defparameter *fold* (fold-priors (list (cons "g-a" *run-a*) (cons "g-b" *run-b*))))

(defun entry (fold harness model tier)
  (find-if (lambda (e) (and (equal (getf e :harness) harness) (equal (getf e :model-ref) model)
                            (equal (getf e :tier) tier)))
           (getf fold :priors)))

(pure-test::check "fold: keys (harness, model_ref, tier), sorted; a node with no tier keys Tier2"
                  (equal (mapcar (lambda (e) (list (getf e :harness) (getf e :model-ref) (getf e :tier))) (getf *fold* :priors))
                         '(("codex" "gpt-x" "Tier1") ("codex" "gpt-x" "Tier2") ("grok" "grok-4" "Tier2")))
                  (getf *fold* :priors))
(pure-test::check "fold: every exclusion reason, each attempt under the first that holds"
                  (equal (getf *fold* :excluded)
                         '(("not_subprocess" . 3) ("model_not_passed" . 2) ("model_unverified" . 2)
                           ("cancelled" . 1) ("no_check_result" . 2)))
                  (getf *fold* :excluded))
(pure-test::check "fold: a failed check counts as a failure, from two graphs"
                  (let ((e (entry *fold* "codex" "gpt-x" "Tier2")))
                    (and (= 6 (getf e :attempts)) (= 5 (getf e :successes))
                         (equal (getf e :provenance)
                                '(("g-a" 1 1 nil) ("g-a" 1 2 t) ("g-a" 4 20 t) ("g-b" 1 1 t) ("g-b" 1 2 t) ("g-b" 1 3 t)))))
                  (entry *fold* "codex" "gpt-x" "Tier2"))
(pure-test::check "fold: counted is the sum of the entries, and a repeated row is dropped once"
                  (and (= 8 (getf *fold* :counted)) (= 1 (getf *fold* :duplicates))
                       (= 8 (reduce #'+ (getf *fold* :priors) :key (lambda (e) (getf e :attempts))))))
(pure-test::check "fold: successes == the references with passed, attempts == their count, in every entry"
                  (every (lambda (e) (and (= (getf e :successes) (count-if #'fourth (getf e :provenance)))
                                          (= (getf e :attempts) (length (getf e :provenance)))))
                         (getf *fold* :priors)))
(pure-test::check "fold: no (graph, task, attempt) reference twice, in an entry or across entries"
                  (let ((refs (loop for e in (getf *fold* :priors)
                                    append (mapcar (lambda (r) (subseq r 0 3)) (getf e :provenance)))))
                    (= (length refs) (length (remove-duplicates refs :test #'equal)))))

(defparameter *time* "2026-11-11T09:30:00Z")
(defparameter *json* (priors-json *fold* *time*))

(pure-test::check "fold: the same input folds to the same bytes, whatever the order or repetition of graphs"
                  (and (string= *json* (priors-json (fold-priors (list (cons "g-b" *run-b*) (cons "g-a" *run-a*))) *time*))
                       (string= *json* (priors-json (fold-priors (list (cons "g-a" *run-a*) (cons "g-b" *run-b*)
                                                                       (cons "g-a" *run-a*)))
                                                    *time*))
                       (string= *json* (priors-json (fold-priors (list (cons "g-a" *run-a*) (cons "g-b" *run-b*))) *time*))))
(pure-test::check "fold: no worker text, description, outcome or evidence reaches the file"
                  (not (search "WORKER-TEXT" *json*)))
(let ((file (nlk:decode-json *json*)))
  (pure-test::check "file: schema, version, generator, source and the v1 entry fields, in order"
                    (and (equal (gethash "schema" file) "meshloop/advisory-priors")
                         (eql (gethash "version" file) 1)
                         (equal (gethash "generated_at" file) *time*)
                         (equal (nlk:json-value file :string "generator" "name") "nodecode-meshloop")
                         (equalp (nlk:json-value file :array "source" "graph_ids") #("g-a" "g-b"))
                         (= 3 (length (gethash "priors" file)))
                         (every (lambda (e) (and (equal (gethash "dispatch_mode" e) "subprocess")
                                                 (= 7 (hash-table-count e))))
                                (gethash "priors" file))
                         (search "\"harness\": \"codex\",
      \"model_ref\": \"gpt-x\",
      \"tier\": \"Tier1\",
      \"dispatch_mode\": \"subprocess\",
      \"attempts\": 1,
      \"successes\": 1,
      \"provenance\": [
        {\"graph_id\": \"g-a\", \"task_id\": 2, \"attempt_id\": 3, \"passed\": true}
      ]" *json*))
                    *json*)
  (pure-test::check "file: every key the file carries equals the fold's, with UTF-8 size under 1 MiB"
                    (and (< (utf-8-length *json*) *priors-file-limit*)
                         (equal (map 'list (lambda (e) (gethash "attempts" e)) (gethash "priors" file)) '(1 6 1)))))
;; For a schema check outside SBCL: MESHLOOP_PRIORS_SAMPLE=FILE writes the proposal there.
(let ((sample (sb-ext:posix-getenv "MESHLOOP_PRIORS_SAMPLE")))
  (when (and sample (plusp (length sample)))
    (with-open-file (out sample :direction :output :if-exists :supersede :external-format :utf-8)
      (write-string *json* out))
    (format t "[note] wrote the fixture proposal to ~a~%" sample)))

(pure-test::check "file: a quote, a backslash and a control character are escaped"
                  (equal (json-quoted (format nil "a\"b\\c~cd" (code-char 1))) "\"a\\\"b\\\\c\\u0001d\""))
(pure-test::check "model_ref predicate: #134's rule, and unverified refused"
                  (and (model-usable-p "gpt-6.1-sol") (model-usable-p "org/model:1")
                       (notany #'model-usable-p (list "" "unverified" "-x" "a b" (format nil "a~cb" #\Tab)
                                                      (make-string 257 :initial-element #\a) nil))))
(pure-test::check "fold: a tier a v1 file cannot carry fails closed"
                  (pure-test::refuses (fold-priors (list (cons "g" (run-json "g" "completed"
                                                                             (list (node-json 1 "Integrated" "\"Tier9\"" (list (attempt-json 1))))))))))
(pure-test::check "fold: an excluded attempt's odd tier does not matter"
                  (null (pure-test::refuses (fold-priors (list (cons "g" (run-json "g" "completed"
                                                                                   (list (node-json 1 "Integrated" "\"Tier9\"" (list (attempt-json 1 :mode "null")))))))))))
(pure-test::check "fold: no attempt id fails closed"
                  (pure-test::refuses (fold-priors (list (cons "g" (nlk:decode-json "{\"nodes\": [{\"id\": 1, \"state\": \"Integrated\", \"tier\": null, \"attempts\": [{\"harness\": \"h\"}]}]}"))))))
(pure-test::check "fold: no run, no prior"
                  (let ((fold (fold-priors '()))) (and (null (getf fold :priors)) (zerop (getf fold :counted)))))

;; An older binary: attempt rows without the #139 fields fail the contract case
;; learn checks (PRIORS-ATTEMPTS refuses through CHECKED), so it writes nothing, while
;; inspect_attempts, which report and accept read, still holds.
(setf *contract* (nlk:decode-json (pure-test::file-text-at (merge-pathnames "contract.json" pure-test::*cell*))))
(let* ((old (nlk:decode-json (format nil "{\"ok\": true, \"command\": \"meshloop:inspect\", \"data\": {\"session_id\": \"g\", \"status\": \"completed\", \"plan_state\": \"PlanAccepted\", \"nodes\": [{\"id\": 1, \"state\": \"Integrated\", \"description\": \"d\", \"attempts\": [{\"attempt_id\": 1, \"harness\": \"h\", \"model_ref\": \"m\", \"outcome\": \"o\", \"evidence\": []}]}]}}")))
       (new (nlk:decode-json (format nil "{\"ok\": true, \"command\": \"meshloop:inspect\", \"data\": {\"session_id\": \"g\", \"status\": \"completed\", \"plan_state\": \"PlanAccepted\", \"nodes\": [{\"id\": 1, \"state\": \"Integrated\", \"description\": \"d\", \"tier\": \"Tier2\", \"attempts\": [~a]}]}}" (attempt-json 1)))))
  (pure-test::check "contract: a pre-#139 answer lacks what learn reads, and learn refuses it"
                    (and (missing-field old (case-fields "priors_attempts"))
                         (null (missing-field old (case-fields "inspect_attempts")))))
  (pure-test::check "contract: a #139 answer has every field learn reads"
                    (null (missing-field new (case-fields "priors_attempts")))
                    (missing-field new (case-fields "priors_attempts"))))
(setf *contract* nil)

;; priors check's answer as the kernel shapes it (crates/meshloop-cli/src/priors.rs).
(let* ((object (nlk:decode-json "{\"ok\": true, \"command\": \"meshloop:priors-check\", \"data\": {\"file\": \"p.json\", \"bytes\": 900, \"priors_sha256\": \"ab12\", \"file_fault\": null, \"evidence_floor\": 5, \"resolved_refs\": 5, \"unresolved_refs\": 0, \"usable\": [{\"index\": 0, \"harness\": \"fixture\", \"model_ref\": \"fixture-model\", \"tier\": \"Tier2\", \"dispatch_mode\": \"subprocess\", \"attempts\": 5, \"successes\": 5, \"pi\": 0.857, \"candidate\": true}], \"ignored\": [{\"index\": 1, \"code\": \"below_evidence_floor\", \"harness\": \"fixture\", \"model_ref\": \"m2\", \"tier\": \"Tier1\", \"dispatch_mode\": \"subprocess\"}], \"diagnostics\": [{\"code\": \"below_evidence_floor\", \"level\": \"entry\", \"entry\": 1, \"harness\": \"fixture\", \"model_ref\": \"m2\", \"tier\": \"Tier1\", \"dispatch_mode\": \"subprocess\", \"detail\": \"1 attempts is below the floor\"}]}}"))
       (text (summarize-priors-check "p.json" object)))
  (pure-test::check "priors check summary: usable entries, ignored count, each diagnostic with its key, the promote command"
                    (and (search "meshloop priors check: p.json: 1 usable, 1 ignored; 1 diagnostic (read-only, nothing promoted)" text)
                         (search "usable fixture / fixture-model / Tier2: 5 attempts, 5 passed" text)
                         (search "below_evidence_floor [entry] fixture / m2 / Tier1: 1 attempts is below the floor" text)
                         (search "meshloop priors accept --file p.json --as <your-name>" text))
                    text))
(let ((text (summarize-priors-check "x.json" (nlk:decode-json "{\"ok\": true, \"command\": \"meshloop:priors-check\", \"data\": {\"file_fault\": \"not_json\", \"usable\": [], \"ignored\": [], \"diagnostics\": [{\"code\": \"not_json\", \"level\": \"file\", \"entry\": null, \"harness\": null, \"model_ref\": null, \"tier\": null, \"dispatch_mode\": null, \"detail\": \"expected value\"}]}}"))))
  (pure-test::check "priors check summary: a file fault, and no promote command for it"
                    (and (search "x.json: file fault not_json, no priors would apply; 0 usable" text)
                         (search "not_json [file]: expected value" text) (not (search "priors accept" text)))
                    text))

;; The digest: pointers and counts only.
(let ((digest (fold-digest *fold* "/h/p.json" "sha256:abc" '(("g-c" . "running, not finished; name it with --graph to fold it anyway"))
                           "promoted priors in effect: none")))
  (dolist (want '("meshloop learn: wrote a routing-prior proposal to /h/p.json (sha256:abc); folded 2 graphs (g-a, g-b): 8 attempts counted, 10 excluded"
                  "prior codex / gpt-x / Tier1: 1 attempt, 1 passed, 0 failed; below the kernel's floor (n >= 5): it will be ignored"
                  "prior codex / gpt-x / Tier2: 6 attempts, 5 passed, 1 failed; clears the kernel's floor (n >= 5)"
                  "prior grok / grok-4 / Tier2: 1 attempt, 1 passed, 0 failed; below the kernel's floor (n >= 5)"
                  "excluded by reason: not_subprocess 3, model_not_passed 2, model_unverified 2, cancelled 1, no_check_result 2"
                  "1 attempt row seen twice under one (graph, task, attempt) dropped"
                  "skipped g-c: running, not finished"
                  "promoted priors in effect: none"
                  "This cell never promotes"
                  "meshloop priors check --file /h/p.json"
                  "meshloop priors accept --file /h/p.json --as <your-name>"))
    (pure-test::check (format nil "digest says: ~a" (subseq want 0 (min 60 (length want)))) (search want digest) digest))
  (pure-test::check "digest: no worker text" (not (search "WORKER-TEXT" digest))))
(let ((digest (fold-digest (fold-priors (list (cons "g" (run-json "g" "completed" (list (node-json 1 "Integrated" "null" (list (attempt-json 1 :passed-model "false")))))))) nil nil nil nil)))
  (pure-test::check "digest: no evidence, no proposal, and no promote command"
                    (and (search "no attempt counts as evidence, so no proposal was written" digest)
                         (search "model_not_passed 1" digest) (not (search "priors accept" digest)))
                    digest))
(pure-test::check "digest: a path with a blank is quoted in the commands"
                  (search "--file '/h/a b.json' --as" (promote-lines "/h/a b.json")))
(pure-test::check "digest: control and bidi characters are not shown"
                  (equal (shown (format nil "a~cb~cc" (code-char #x202e) (code-char 7))) "abc"))

;; learn's words.
(pure-test::check "learn words: --graph repeated and bare graph ids, --out"
                  (multiple-value-bind (graphs out) (learn-words "--graph g1 g2 --out .agent-runs/p.json --graph g3")
                    (and (equal graphs '("g1" "g2" "g3")) (equal out ".agent-runs/p.json"))))
(dolist (case '("--force" "--graph" "--graph --x" "--out a.json --out b.json" "--out"))
  (pure-test::check (format nil "learn words refused: ~a" case) (pure-test::refuses (learn-words case))))

;; --out: a new .json file inside the repository's .agent-runs/, links resolved.
(let* ((base (format nil "/tmp/nodecode-meshloop-pure-~d/" (random 1000000000 (make-random-state t))))
       (root (concatenate 'string base "repo/")))
  (ensure-directories-exist (concatenate 'string root ".git/"))
  (ensure-directories-exist (concatenate 'string base "elsewhere/"))
  (unwind-protect
       (progn
         (pure-test::check "--out: the check learn makes before reading meshloop makes no folder"
                           (and (search "/repo/.agent-runs/early/p.json" (out-file root ".agent-runs/early/p.json"))
                                (not (probe-file (concatenate 'string root ".agent-runs/")))
                                (pure-test::refuses (out-file root "../x.json"))))
         (pure-test::check "--out: .agent-runs/priors/p.json is taken, its folder made"
                           (let ((path (agent-runs-path root ".agent-runs/priors/p.json")))
                             (and (search "/repo/.agent-runs/priors/p.json" path)
                                  (probe-file (concatenate 'string root ".agent-runs/priors/")))))
         (dolist (case (list "../x.json" ".agent-runs/../x.json" "x.json" ".agent-runs/p.txt" "-x.json"
                             (concatenate 'string base "elsewhere/x.json") "/etc/x.json"))
           (pure-test::check (format nil "--out refused: ~a" case) (pure-test::refuses (agent-runs-path root case))))
         (with-open-file (out (concatenate 'string root ".agent-runs/taken.json") :direction :output) (write-string "{}" out))
         (pure-test::check "--out refused: a file that exists"
                           (pure-test::refuses (agent-runs-path root ".agent-runs/taken.json")))
         (sb-ext:run-program "/bin/ln" (list "-s" (concatenate 'string base "elsewhere") (concatenate 'string root ".agent-runs/out")))
         (pure-test::check "--out refused: a folder linked outside .agent-runs/, and nothing made there"
                           (and (pure-test::refuses (agent-runs-path root ".agent-runs/out/sub/x.json"))
                                (not (probe-file (concatenate 'string base "elsewhere/sub/"))))))
    (sb-ext:run-program "/bin/rm" (list "-rf" base))))

;;; --- the :tool veto on priors accept and priors revoke (#139) -----------------------

(dolist (case '(("priors accept, shell string" "(sh \"meshloop priors accept --file .agent-runs/p.json --as me\")")
                ("priors revoke, shell string" "(sh \"meshloop priors revoke --as me\")")
                ("priors revoke, quoted argv" "(uiop:run-program '(\"meshloop\" \"priors\" \"revoke\" \"--as\" \"me\"))")
                ("priors accept, argv list" "(sb-ext:run-program \"/x/target/debug/meshloop\" (list \"priors\" \"accept\" \"--file\" \"p.json\"))")
                ("priors accept, python argv" "subprocess.run([\"meshloop\", \"priors\", \"accept\", \"--file\", \"p.json\"])")
                ("priors revoke after a flag" "(sh \"meshloop priors --json revoke --as me\")")
                ("priors revoke, upper case" "(SH \"MESHLOOP PRIORS REVOKE --AS ME\")")
                ("priors revoke, slash line" "(post \"/api/gateway/slash\" \"{\\\"line\\\":\\\"/meshloop priors revoke --as me\\\"}\")")
                ("priors accept, prefixed slash" "(post \"/meshloop:priors accept --file p.json\")")
                ("priors revoke piped into xargs" "(sh \"echo revoke | xargs meshloop priors --as me\")")
                ("priors revoke, executable in a variable" "(sh \"B=meshloop; $B priors revoke --as me\")")
                ("priors revoke, cargo run -p" "(sh \"cargo run -q -p meshloop-cli -- priors revoke --as me\")")
                ("priors revoke, spliced" "sh -c 'meshloop priors rev''oke --as me'")
                ("priors revoke, \\u escape" "(sh \"meshloop priors \\u0072evoke --as me\")")
                ("priors accept, a wrapper's command" "(sh \"timeout 5 meshloop priors accept --file p.json --as me\")")
                ("priors accept, MCP tool name" "(mcp-call \"meshloop_priors_accept\" (list))")))
  (pure-test::check (format nil "veto refuses: ~a" (first case)) (pure-test::veto (second case)) (second case)))
(dolist (case '(("priors show" "(sh \"meshloop priors show --json\")")
                ("priors check" "(sh \"meshloop priors check --file .agent-runs/p.json --json\")")
                ("priors check, argv" "(uiop:run-program '(\"meshloop\" \"priors\" \"check\" \"--file\" \"p.json\"))")
                ("priors show, slash line" "(post \"/api/gateway/slash\" \"{\\\"line\\\":\\\"/meshloop priors show\\\"}\")")
                ("/meshloop learn" "(post \"/api/gateway/slash\" \"{\\\"line\\\":\\\"/meshloop learn --graph g1\\\"}\")")
                ("the words in a message" "(sh \"git commit -m 'revoke the stale priors docs'\")")
                ("priors show, then a commit" "(sh \"meshloop priors show; git commit -m revoke\")")))
  (pure-test::check (format nil "veto lets through: ~a" (first case)) (null (pure-test::veto (second case)))
                    (pure-test::veto (second case))))
(pure-test::check "veto refuses MCP priors tools by name and by last __ segment"
                  (every (lambda (name) (gated-call name (make-hash-table)))
                         '("mcp__meshloop__meshloop_priors_revoke" "mcp__meshloop__priors_accept" "mcp__meshloop__priors_revoke")))
(pure-test::check "veto lets MCP priors show and another server's priors_accept through"
                  (notany (lambda (name) (gated-call name (make-hash-table)))
                          '("mcp__meshloop__priors_show" "mcp__meshloop__priors_check" "mcp__github__priors_accept")))

;;; Read from the source: learn asks meshloop only for read verbs, and for priors
;;; only show and check; no source runs priors accept or revoke.
(let ((learn (pure-test::file-text "learn")))
  (pure-test::check "learn runs only meshloop's read verbs: inspect, status, workspace inventory, priors show|check"
                    (and (not (search "(act " learn))
                         (loop for start = (search "(ask \"" learn) then (search "(ask \"" learn :start2 (1+ start))
                               while start
                               always (let ((text (subseq learn (+ start 5) (min (length learn) (+ start 40)))))
                                        (some (lambda (ok) (eql 0 (search ok text)))
                                              '("\"inspect\" (list \"--attempts\"" "\"status\" '()" "\"workspace\" (list \"inventory\""
                                                "\"priors\" (list \"show\"" "\"priors\" (list \"check\"")))))))
(pure-test::check "no source passes accept or revoke to a priors call"
                  (let ((all (format nil "~{~a~}" (mapcar #'pure-test::file-text pure-test::*files*))))
                    (notany (lambda (form) (search form all))
                            '("\"priors\" (list \"accept\"" "\"priors\" (list \"revoke\"" "(list \"priors\" \"accept\""
                              "(list \"priors\" \"revoke\"" "(act \"priors\"" "'(\"priors\" \"accept\"" "'(\"priors\" \"revoke\""))))

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
