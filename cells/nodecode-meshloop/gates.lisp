;;;; gates.lisp --- the :tool veto on meshloop's human gates.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; ADR 0038: accept, review-plan and integrate are the operator's to type, as
;;;; /meshloop slash commands, and are never model-callable. This cell installs
;;;; one hook on nodecode's :TOOL point that refuses an organism tool call whose
;;;; text runs one of them: `meshloop accept ...' in a shell string or an argv
;;;; list, the MCP tool meshloop_accept, a call into this cell's own code, or the
;;;; /meshloop slash line itself.
;;;;
;;;; DEFENCE IN DEPTH, NOT A SECURITY BOUNDARY. The organism's eval has the
;;;; operator's full host authority: a call can build the command from pieces,
;;;; read it from a file, or reach the gateway another way, and this reading
;;;; will not see it. The hook makes the honest path the easy one and records a
;;;; refusal the model can read; the kernel cannot tell who typed `--as' (ADR
;;;; 0038, Consequences), so acceptance evidence stays a claim.
;;;;
;;;; The reading: every string the call's arguments carry (eval's form, its
;;;; strings object, any other argument) is cut into tokens at blanks, quotes,
;;;; brackets and shell separators. A token naming the meshloop executable
;;;; (meshloop, /path/to/meshloop, meshloop.exe, /meshloop) is followed, within a
;;;; few tokens and stepping over flags, keywords, format directives and Lisp
;;;; list constructors, by its verb; a gated verb there is refused. A verb is
;;;; also read in its prefixed form (meshloop:accept). The MCP tool names and
;;;; this cell's package reached with :: are refused wherever they appear.

(in-package #:nodecode-meshloop)

(defparameter +gated-verbs+ '("accept" "review-plan" "integrate")
  "The meshloop verbs that record a human decision: never a tool call's.")

(defparameter +gated-tools+ '("meshloop_accept" "meshloop_review_plan" "meshloop_integrate")
  "The same verbs as `meshloop mcp' names its tools.")

(defparameter +internal-prefixes+ '("meshloop::" "nodecode-meshloop::")
  "A token reaching this cell's own unexported code: the slash commands' functions.")

(defparameter +token-breaks+
  (format nil " ~c~c~c~c\"'`()[]{};|&,=\\<>" #\Tab #\Newline #\Return #\Page)
  "What separates one token of a call's text from the next.")

(defparameter +stepped-over+ '("list" "list*" "vector" "quote" "append" "#" "cons")
  "Tokens between the executable and its verb that build an argv list in Lisp.")

(defparameter *verb-reach* 6
  "How many tokens after the executable its verb may sit.")

(defun call-strings (value)
  "Every string VALUE carries: itself, a hash table's values, an array's or a
list's elements, at any depth."
  (let ((out '()))
    (labels ((walk (x)
               (cond ((stringp x) (push x out))
                     ((hash-table-p x) (loop for v being the hash-values of x do (walk v)))
                     ((vectorp x) (loop for v across x do (walk v)))
                     ((consp x) (walk (car x)) (walk (cdr x))))))
      (walk value))
    (nreverse out)))

(defun tokens (text)
  "TEXT cut at +TOKEN-BREAKS+, each token lowercased, empty ones dropped."
  (let ((out '()) (start 0))
    (loop for i from 0 to (length text)
          do (when (or (= i (length text)) (find (char text i) +token-breaks+))
               (when (> i start)
                 (push (string-downcase (subseq text start i)) out))
               (setf start (1+ i))))
    (nreverse out)))

(defun executable-p (token)
  "Whether TOKEN names the meshloop executable: its last path segment is
meshloop or meshloop.exe (so /meshloop, the slash line, is one too)."
  (let* ((slash (position #\/ token :from-end t))
         (base (if slash (subseq token (1+ slash)) token)))
    (member base '("meshloop" "meshloop.exe") :test #'string=)))

(defun verb-of (token)
  "TOKEN as a meshloop verb: a leading / and a meshloop: prefix taken off."
  (let* ((bare (string-left-trim "/" token))
         (prefix "meshloop:"))
    (if (and (> (length bare) (length prefix)) (string= prefix bare :end2 (length prefix)))
        (subseq bare (length prefix))
        bare)))

(defun gated-verb-p (token)
  "Whether TOKEN, as a verb, is one of +GATED-VERBS+."
  (member (verb-of token) +gated-verbs+ :test #'string=))

(defun stepped-over-p (token)
  "Whether TOKEN may sit between the executable and its verb: a flag, a
keyword, a format directive or a list constructor."
  (or (find (char token 0) "-:~")
      (member token +stepped-over+ :test #'string=)))

(defun gated-in-text (text)
  "Why TEXT runs a gated meshloop verb, or NIL."
  (let ((tokens (coerce (tokens text) 'vector)))
    (loop for i from 0 below (length tokens)
          for token = (aref tokens i)
          do (cond ((find-if (lambda (tool) (search tool token)) +gated-tools+)
                    (return (format nil "it names the MCP tool ~a" token)))
                   ((find-if (lambda (prefix) (and (> (length token) (length prefix))
                                                   (string= prefix token :end2 (length prefix))))
                             +internal-prefixes+)
                    (return (format nil "it reaches into this cell's own code (~a)" token)))
                   ((and (char= #\/ (char token 0)) (string/= (verb-of token) token)
                         (gated-verb-p token))
                    (return (format nil "it types the slash command ~a" token)))
                   ((executable-p token)
                    (let ((verb (loop for j from (1+ i) below (min (length tokens) (+ i 1 *verb-reach*))
                                      for next = (aref tokens j)
                                      unless (stepped-over-p next) return next)))
                      (when (and verb (gated-verb-p verb))
                        (return (format nil "it runs meshloop ~a" (verb-of verb))))))))))

(defun gated-call (name arguments)
  "Why a tool call NAME with ARGUMENTS runs one of meshloop's human gates, or NIL."
  (or (and (stringp name)
           (let ((tool (find-if (lambda (tool) (search tool (string-downcase name))) +gated-tools+)))
             (and tool (format nil "the tool ~a is meshloop ~a" name (verb-of (substitute #\- #\_ (subseq tool 9)))))))
      (loop for text in (call-strings arguments)
            thereis (gated-in-text text))))

(defun veto-gated-calls (op next)
  "The :TOOL point. OP is (:name TOOL :arguments HASH :call-id ID). A call that
runs accept, review-plan or integrate answers a refusal, recorded as the call's
failure, and NEXT never runs; every other call runs as it would."
  (let ((reason (gated-call (getf op :name) (getf op :arguments))))
    (if reason
        (nle:failure
         (format nil "ERROR: refused by nodecode-meshloop: ~a. Accept, review-plan and integrate ~
                      record a person's decision, so the operator types them as /meshloop commands, ~
                      naming who decides with --as. The call did not run: tell the operator the ~
                      command instead, and do not retry it another way" reason))
        (funcall next op))))

(defun hook-installed-p ()
  "Whether VETO-GATED-CALLS is on the organism's :TOOL chain now."
  ;; The chain is the kernel's own and not exported; looked up, so a release
  ;; without it answers NIL rather than failing to load.
  (let ((symbol (find-symbol "*HOOKS*" '#:nodecode.kernel)))
    (and symbol (boundp symbol)
         (loop for entry in (cdr (assoc :tool (symbol-value symbol) :test #'equal))
               thereis (eq (cdr entry) #'veto-gated-calls)))))

(defun hook-check-of (text)
  "/meshloop hook-check FORM: whether the :tool hook refuses an eval call whose
form is FORM. It runs the installed hook function on that call with a NEXT that
only notes it was reached; FORM itself is never read as code or run."
  (answering "hook-check"
             (lambda ()
               (when (zerop (length (nlk:trimmed (or text ""))))
                 (fail "needs the text of a call: /meshloop hook-check (sh \"meshloop accept --task 1 --as me\")"))
               (let* ((arguments (make-hash-table :test #'equal))
                      (reached nil))
                 (setf (gethash "form" arguments) text)
                 (let ((answer (veto-gated-calls (list :name "eval" :arguments arguments :call-id "hook-check")
                                                 (lambda (op) (declare (ignore op)) (setf reached t) "ran"))))
                   (format nil "meshloop hook-check: ~:[refused: ~a~;~*passes: the hook lets this call run~]; ~
                                hook ~:[NOT on~;on~] this organism's :tool chain"
                           reached (nlk:one-line answer :cap (* 3 *text-cap*))
                           (hook-installed-p)))))))
