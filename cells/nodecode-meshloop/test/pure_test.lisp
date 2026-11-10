;;;; pure_test.lisp --- an offline check of the cell under a plain SBCL.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;;   sbcl --script cells/nodecode-meshloop/test/pure_test.lisp
;;;;
;;;; No nodecode, no socket, no model. It reads every form of package.lisp,
;;;; cell.lisp, gates.lisp and drive.lisp (a reader error, an unbalanced
;;;; parenthesis, fails here), compiles every DEFUN, DEFPARAMETER and DEFVAR
;;;; among them against stand-in NLK and NLE packages (a full compiler warning,
;;;; such as a call with the wrong number of arguments, fails here), and then
;;;; tests the functions that need nothing of nodecode: the :tool veto's
;;;; reading of a call, and the argument checks of the commands that write
;;;; (identity, task, graph, branch, plan path, flags). The load test
;;;; (load_test.py) is what runs the cell in nodecode. Optional: neither
;;;; `cargo test' nor `xtask check' runs this. Exit status 0 means every check held.

(require :asdf) ; the cell reads uiop and asdf, as nodecode's image has them

(defpackage #:nodecode.kernel (:use #:cl) (:nicknames #:nlk))
(defpackage #:nodecode.evolved (:use #:cl) (:nicknames #:nle))

(defpackage #:pure-test (:use #:cl))
(in-package #:pure-test)

(defparameter *cell* (merge-pathnames "../" (make-pathname :name nil :type nil
                                                           :defaults *load-truename*)))
(defparameter *files* '("package" "cell" "gates" "drive"))
(defparameter *failures* 0)
(defparameter *checks* 0)

(defun check (name ok &optional detail)
  (incf *checks*)
  (unless ok (incf *failures*))
  (format t "[~:[FAIL~;pass~]] ~a~@[: ~a~]~%" ok name (and (not ok) detail)))

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

(in-package #:pure-test)
(format t "~d of ~d checks passed~%" (- *checks* *failures*) *checks*)
(sb-ext:exit :code (if (zerop *failures*) 0 1))
