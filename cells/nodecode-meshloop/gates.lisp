;;;; gates.lisp --- the :tool veto on meshloop's human gates.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; ADR 0038: accept, review-plan and integrate are the operator's to type, as
;;;; /meshloop slash commands, and are never model-callable; so are the two-word
;;;; verbs priors accept and priors revoke (#139), which promote and revoke
;;;; routing priors and which this cell never runs at all. This cell installs
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
;;;; The reading. Every string the call's arguments carry (eval's form, its
;;;; strings object, any other argument) is read as it is and with \uXXXX and
;;;; %XX decoded, two ways:
;;;;
;;;;  - As tokens, cut at blanks, quotes, brackets and shell separators: a token
;;;;    naming the meshloop executable followed, within a few tokens and stepping
;;;;    over flags, keywords, format directives and Lisp list constructors, by a
;;;;    gated verb, or by priors and then accept or revoke (an argv list in Lisp
;;;;    or Python); a token with the MCP tool
;;;;    name; this cell's package reached with ::; or a find-symbol, intern,
;;;;    find-package or in-package form naming the cell's package.
;;;;  - As shell lines, the string itself and each double-quoted string in it:
;;;;    words joined across quotes with backslashes dropped (m""eshloop is
;;;;    meshloop), commands cut at ; && || & and newlines, a | pipeline kept
;;;;    whole. A command whose words hold a gated verb anywhere (or priors and
;;;;    accept or revoke anywhere) and a meshloop
;;;;    runner (the executable or a $VARIABLE in command position, which a
;;;;    wrapper such as xargs, env, sudo or timeout passes on, or meshloop-cli
;;;;    after -p) is refused: `echo accept | xargs meshloop' and `B=meshloop;
;;;;    $B accept' among them. A word that still holds a blank once its quotes
;;;;    are joined is read again as a line (sh -c 'meshloop acce''pt').
;;;;
;;;; The meshloop binary takes its verb as the first argument and refuses a
;;;; flag there, so `meshloop --task 1 accept' is no way around it. A command
;;;; assembled from pieces at run time is: the reading sees text, not values.

(in-package #:nodecode-meshloop)

(defparameter +gated-verbs+ '("accept" "review-plan" "integrate")
  "The meshloop verbs that record a human decision: never a tool call's.")

(defparameter +gated-pairs+ '(("priors" "accept") ("priors" "revoke"))
  "The two-word meshloop verbs that record a person's decision (ADR 0038, #139):
promoting routing priors and revoking them. priors show and priors check only
read, and pass.")

(defparameter +gated-tools+ '("meshloop_accept" "meshloop_review_plan" "meshloop_integrate"
                              "meshloop_priors_accept" "meshloop_priors_revoke")
  "The same verbs as `meshloop mcp' would name its tools. No MCP tool promotes
priors (ADR 0038); the last two are refused should one ever be named so.")

(defparameter +internal-prefixes+ '("meshloop::" "nodecode-meshloop::")
  "A token reaching this cell's own unexported code: the slash commands' functions.")

(defparameter +package-names+ '("meshloop" "nodecode-meshloop")
  "This cell's package, by its nickname and its name.")

(defparameter +symbol-finders+ '("find-symbol" "find-symbol*" "intern" "find-package" "in-package")
  "Operators that reach a package's symbols by name.")

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

(defun hex-at (text start count)
  "The number COUNT hex digits of TEXT at START spell, or NIL."
  (and (<= (+ start count) (length text))
       (every (lambda (c) (digit-char-p c 16)) (subseq text start (+ start count)))
       (parse-integer text :start start :end (+ start count) :radix 16)))

(defun decoded (text)
  "TEXT with \\uXXXX escapes and %XX URL escapes turned into the characters they name."
  (with-output-to-string (out)
    (let ((i 0) (n (length text)))
      (loop while (< i n)
            do (let* ((c (char text i))
                      (u (and (char= c #\\) (< (1+ i) n) (char-equal (char text (1+ i)) #\u)
                              (hex-at text (+ i 2) 4)))
                      (p (and (char= c #\%) (hex-at text (1+ i) 2))))
                 (cond (u (write-char (code-char u) out) (incf i 6))
                       (p (write-char (code-char p) out) (incf i 3))
                       (t (write-char c out) (incf i))))))))

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
  (member (base-name token) '("meshloop" "meshloop.exe") :test #'string-equal))

(defun verb-of (token)
  "TOKEN as a meshloop verb: a leading / and a meshloop: prefix taken off."
  (let* ((bare (string-downcase (string-left-trim "/" token)))
         (prefix "meshloop:"))
    (if (and (> (length bare) (length prefix)) (string= prefix bare :end2 (length prefix)))
        (subseq bare (length prefix))
        bare)))

(defun gated-verb-p (token)
  "Whether TOKEN, as a verb, is one of +GATED-VERBS+."
  (member (verb-of token) +gated-verbs+ :test #'string=))

(defun gated-pair (first second)
  "\"priors accept\" or \"priors revoke\" when FIRST and SECOND, as verbs, are
one of +GATED-PAIRS+; NIL otherwise, and for priors show or priors check."
  (and first second
       (find-if (lambda (pair) (and (string= (verb-of first) (first pair))
                                    (string= (verb-of second) (second pair))))
                +gated-pairs+)
       (format nil "~a ~a" (verb-of first) (verb-of second))))

(defun joined-pair-p (verb)
  "Whether VERB is a gated pair written as one word: priors-accept, priors_revoke."
  (find-if (lambda (pair)
             (member (verb-of verb)
                     (list (format nil "~a-~a" (first pair) (second pair))
                           (format nil "~a_~a" (first pair) (second pair)))
                     :test #'string=))
           +gated-pairs+))

(defun stepped-over-p (token)
  "Whether TOKEN may sit between the executable and its verb: a flag, a
keyword, a format directive or a list constructor."
  (or (find (char token 0) "-:~")
      (member token +stepped-over+ :test #'string=)))

(defun package-reference-p (token)
  "Whether TOKEN, as the text wrote it, names this cell's package as a
package designator: :meshloop, #:meshloop, MESHLOOP in capitals, or
nodecode-meshloop in any case."
  (let ((bare (string-left-trim "#:" token)))
    (and (member bare +package-names+ :test #'string-equal)
         (or (string/= bare token)
             (string-equal bare "nodecode-meshloop")
             (string= bare (string-upcase bare))))))

(defun finder-p (token)
  "Whether TOKEN is one of +SYMBOL-FINDERS+, package prefix and all."
  (let ((colon (position #\: token :from-end t)))
    (member (if colon (subseq token (1+ colon)) token) +symbol-finders+ :test #'string-equal)))

(defun next-verb-index (tokens from)
  "The index of the first of TOKENS, a vector, at or after FROM and within
*VERB-REACH* of it, that is not stepped over; NIL when there is none."
  (loop for j from from below (min (length tokens) (+ from *verb-reach*))
        unless (stepped-over-p (aref tokens j)) return j))

(defun gated-from (tokens j)
  "What the verb at index J of TOKENS runs when it is gated: a gated verb, or a
gated pair it starts with the next verb after it; NIL otherwise."
  (let ((verb (aref tokens j)))
    (if (gated-verb-p verb)
        (verb-of verb)
        (let ((k (next-verb-index tokens (1+ j))))
          (and k (gated-pair verb (aref tokens k)))))))

(defun token-reading (text)
  "Why TEXT, read as tokens, runs a gated meshloop verb, or NIL."
  (let* ((cased (tokens-cased text))
         (tokens (coerce (mapcar #'string-downcase cased) 'vector)))
    (or (and (some #'finder-p cased) (find-if #'package-reference-p cased)
             (format nil "it reaches this cell's package by name (~a)" (find-if #'package-reference-p cased)))
        (loop for i from 0 below (length tokens)
              for token = (aref tokens i)
              do (cond ((find-if (lambda (tool) (search tool token)) +gated-tools+)
                        (return (format nil "it names the MCP tool ~a" token)))
                       ((find-if (lambda (prefix) (and (> (length token) (length prefix))
                                                       (string= prefix token :end2 (length prefix))))
                                 +internal-prefixes+)
                        (return (format nil "it reaches into this cell's own code (~a)" token)))
                       ((and (char= #\/ (char token 0)) (string/= (verb-of token) token)
                             (gated-from tokens i))
                        (return (format nil "it types the slash command ~a" (gated-from tokens i))))
                       ((executable-p token)
                        (let* ((j (next-verb-index tokens (1+ i)))
                               (gated (and j (gated-from tokens j))))
                          (when gated
                            (return (format nil "it runs meshloop ~a" gated))))))))))

(defun tokens-cased (text)
  "TEXT cut as TOKENS cuts it, case kept."
  (let ((out '()) (start 0))
    (loop for i from 0 to (length text)
          do (when (or (= i (length text)) (find (char text i) +token-breaks+))
               (when (> i start) (push (subseq text start i) out))
               (setf start (1+ i))))
    (nreverse out)))

;;; --- shell lines ------------------------------------------------------------------

(defun shell-commands (text)
  "TEXT read as a shell line: its commands, each a list of words with | kept
between the commands of a pipeline. Quotes join the pieces of a word and are
dropped, a backslash is dropped and the character after it kept, and ; && ||
& and newlines end a command."
  (let ((commands '()) (pipeline '()) (word (make-string-output-stream))
        (in-word nil) (quote nil) (i 0) (n (length text)))
    (flet ((end-word ()
             (when in-word
               (push (get-output-stream-string word) pipeline)
               (setf in-word nil)))
           (next-is (c) (and (< (1+ i) n) (char= (char text (1+ i)) c))))
      (flet ((end-command ()
               (end-word)
               (when pipeline (push (nreverse pipeline) commands))
               (setf pipeline '())))
        (loop while (< i n)
              do (let ((c (char text i)))
                   (cond ((eql quote #\')
                          (if (char= c #\') (setf quote nil) (write-char c word))
                          (incf i))
                         ((eql quote #\")
                          (cond ((char= c #\") (setf quote nil))
                                ((and (char= c #\\) (< (1+ i) n)) (incf i) (write-char (char text i) word))
                                (t (write-char c word)))
                          (incf i))
                         ((char= c #\\)
                          (when (< (1+ i) n)
                            (write-char (char text (1+ i)) word)
                            (setf in-word t))
                          (incf i 2))
                         ((find c "'\"") (setf quote c in-word t) (incf i))
                         ((find c '(#\Space #\Tab #\Return #\Page)) (end-word) (incf i))
                         ((or (char= c #\Newline) (char= c #\;)) (end-command) (incf i))
                         ((char= c #\&) (end-command) (incf i (if (next-is #\&) 2 1)))
                         ((char= c #\|)
                          (if (next-is #\|)
                              (progn (end-command) (incf i 2))
                              (progn (end-word) (push "|" pipeline) (incf i))))
                         (t (write-char c word) (setf in-word t) (incf i)))))
        (end-command)))
    (nreverse commands)))

(defun double-quoted (text)
  "The double-quoted strings in TEXT, their backslash escapes read."
  (let ((out '()) (i 0) (n (length text)))
    (loop while (< i n)
          do (if (char= (char text i) #\")
                 (let ((s (make-string-output-stream)))
                   (incf i)
                   (loop while (and (< i n) (char/= (char text i) #\"))
                         do (when (and (char= (char text i) #\\) (< (1+ i) n)) (incf i))
                            (write-char (char text i) s)
                            (incf i))
                   (push (get-output-stream-string s) out)
                   (incf i))
                 (incf i)))
    (nreverse out)))

(defun word-core (word)
  "WORD without the brackets and commas a Lisp or Python form leaves on it."
  (string-trim "()[]{},`" word))

(defun assignment-p (word)
  "Whether WORD is NAME=VALUE, a shell assignment before a command."
  (let ((eq (position #\= word)))
    (and eq (plusp eq)
         (every (lambda (c) (or (alphanumericp c) (char= c #\_))) (subseq word 0 eq)))))

(defparameter +wrappers+ '("xargs" "exec" "env" "command" "nice" "timeout" "sudo" "nohup" "time")
  "Commands that run the command after them: its first word is in command position too.")

(defun base-name (word)
  "WORD's last path segment."
  (let ((slash (position-if (lambda (c) (find c "/\\")) word :from-end t)))
    (if slash (subseq word (1+ slash)) word)))

(defun command-heads (words)
  "The words in command position: the first of each command of a pipeline,
assignments before it stepped over, and the command a wrapper (xargs, env,
sudo, timeout ...) runs, the wrapper's options, numbers and timeout's duration
stepped over."
  (let ((heads '()) (expect t) (wrapped nil) (duration nil))
    (dolist (word words (nreverse heads))
      (cond ((string= word "|") (setf expect t wrapped nil duration nil))
            ((not expect))
            ((assignment-p word))
            ((and wrapped (or (char= #\- (char word 0)) (every #'digit-char-p word))))
            (duration (setf duration nil))
            (t (push word heads)
               (let ((wrapper (find (string-downcase (base-name word)) +wrappers+ :test #'string=)))
                 (setf expect (and wrapper t)
                       wrapped (and wrapper t)
                       duration (equal wrapper "timeout"))))))))

(defun meshloop-runner (words)
  "What in WORDS runs meshloop: the executable or a $VARIABLE in command
position, or meshloop-cli after cargo's -p; NIL when nothing does. The
executable elsewhere in a command (a folder named meshloop that git or ls
reads) runs nothing."
  (let ((heads (command-heads words)))
    (or (find-if #'executable-p heads)
        (find-if (lambda (head) (char= #\$ (char head 0))) heads)
        (loop for (flag value) on words
              thereis (cond ((and (member flag '("-p" "--package") :test #'string=)
                                  (string-equal value "meshloop-cli"))
                             (format nil "~a ~a" flag value))
                            ((member flag '("-pmeshloop-cli" "--package=meshloop-cli") :test #'string-equal)
                             flag))))))

(defun shell-verb (words)
  "The gated verb WORDS hold anywhere: a gated pair (priors and accept or
revoke, in any order, as xargs may append the second), else a gated verb;
NIL when they hold none."
  (or (loop for (head second) in +gated-pairs+
            thereis (and (find head words :key #'verb-of :test #'string=)
                         (find second words :key #'verb-of :test #'string=)
                         (format nil "~a ~a" head second)))
      (let ((verb (find-if #'gated-verb-p words)))
        (and verb (verb-of verb)))))

(defun shell-reading (text &optional (depth 3))
  "Why TEXT, read as shell lines, runs a gated meshloop verb, or NIL. A word
that still holds a blank once its quotes are joined (sh -c 'meshloop acce''pt',
eval \"mesh\"\"loop ...\") is read again as a line of its own, DEPTH deep."
  (loop for line in (cons text (double-quoted text))
        thereis (loop for command in (shell-commands line)
                      for words = (remove "" (mapcar #'word-core command) :test #'string=)
                      for verb = (shell-verb words)
                      for runner = (and verb (meshloop-runner words))
                      when runner
                        return (format nil "it runs ~a with ~a in one command" runner verb)
                      when (plusp depth)
                        do (let ((inner (loop for word in words
                                              for blank = (find-if (lambda (c) (find c '(#\Space #\Tab #\Newline))) word)
                                              for reason = (and blank (shell-reading word (1- depth)))
                                              when reason return reason)))
                             (when inner (return inner))))))

(defun gated-in-text (text)
  "Why TEXT runs a gated meshloop verb, or NIL."
  (loop for each in (remove-duplicates (list text (decoded text)) :test #'string=)
        thereis (or (token-reading each) (shell-reading each))))

(defun mcp-gated (name)
  "Why tool NAME is a gated meshloop verb by its MCP name, or NIL: it holds
meshloop_accept and the like, or its server segment is meshloop and its last
__ segment a gated verb (mcp__meshloop__accept)."
  (let* ((lower (string-downcase name))
         (tool (find-if (lambda (tool) (search tool lower)) +gated-tools+)))
    (if tool
        (format nil "the tool ~a is meshloop ~a" name
                (substitute (if (search "priors_" tool) #\Space #\-) #\_ (subseq tool 9)))
        (let ((segments (loop with start = 0
                              for at = (search "__" lower :start2 start)
                              collect (subseq lower start at)
                              while at
                              do (setf start (+ at 2)))))
          (when (and (rest segments)
                     (some (lambda (segment) (search "meshloop" segment)) (butlast segments)))
            (let ((verb (substitute #\- #\_ (car (last segments)))))
              (cond ((gated-verb-p verb)
                     (format nil "the tool ~a is meshloop ~a" name (verb-of verb)))
                    ((joined-pair-p verb)
                     (format nil "the tool ~a is meshloop ~a" name (substitute #\Space #\- (verb-of verb)))))))))))

(defun gated-call (name arguments)
  "Why a tool call NAME with ARGUMENTS runs one of meshloop's human gates, or NIL."
  (or (and (stringp name) (mcp-gated name))
      (loop for text in (call-strings arguments)
            thereis (gated-in-text text))))

(defun veto-gated-calls (op next)
  "The :TOOL point. OP is (:name TOOL :arguments HASH :call-id ID). A call that
runs accept, review-plan, integrate, priors accept or priors revoke answers a
refusal, recorded as the call's failure, and NEXT never runs; every other call
runs as it would."
  (let ((reason (gated-call (getf op :name) (getf op :arguments))))
    (if reason
        (nle:failure
         (format nil "ERROR: refused by nodecode-meshloop: ~a. Accept, review-plan, integrate, ~
                      priors accept and priors revoke record a person's decision, so the operator ~
                      types them, naming who decides with --as. The call did not run: tell the ~
                      operator the command instead, and do not retry it another way" reason))
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
