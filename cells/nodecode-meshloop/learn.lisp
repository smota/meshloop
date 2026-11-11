;;;; learn.lisp --- /meshloop learn: a routing-prior proposal and its digest (#139).
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; ADR 0038, amendment 2026-11-10 (#139), and learning-plane.md, "Advisory
;;;; input". /meshloop learn folds what meshloop recorded of finished attempts
;;;; into a proposal file of routing priors (schemas/advisory-priors.v1.schema.json)
;;;; and answers a digest: pointers and counts only.
;;;;
;;;; The fold is deterministic code here, never a model turn. It reads attempt
;;;; fields meshloop wrote (harness, model_ref, dispatch_mode, model_passed,
;;;; check_result, attempt_id), the node's id, tier and state, and nothing else:
;;;; never worker output, model-review verdicts, evidence summaries, node
;;;; descriptions or Board posts. An attempt counts only when it was dispatched
;;;; as a subprocess, meshloop passed its model on argv, the model is known, its
;;;; task is not Cancelled and it has a deterministic check result; every other
;;;; attempt is reported under one reason. The same input folds to the same
;;;; bytes, but for generated_at.
;;;;
;;;; Read-only with respect to meshloop: the cell runs inspect, status,
;;;; workspace inventory, priors show and priors check, all of which only read.
;;;; It writes the proposal file and nothing else: under this cell's own state
;;;; in the nodecode home, or at a path the operator types inside the
;;;; repository's .agent-runs/. A proposal has no effect until a person promotes
;;;; it with `meshloop priors accept --file F --as <name>' in a shell. The cell
;;;; never promotes or revokes, has no command that does, and its :tool veto
;;;; refuses a tool call that would (gates.lisp). The evidence floor (n >= 5)
;;;; is the kernel's; the digest only says which entries clear it.

(in-package #:nodecode-meshloop)

;;; --- the v1 file's bounds ----------------------------------------------------------

(defparameter +exclusion-reasons+
  '("not_subprocess" "model_not_passed" "model_unverified" "cancelled" "no_check_result")
  "Why an attempt is no evidence, in the order the fold tests them: the first
that holds is the attempt's one reason.")

(defparameter +tiers+ '("Tier1" "Tier2" "Tier3")
  "The tiers a v1 entry may carry, as meshloop names them.")

(defparameter +default-tier+ "Tier2"
  "The tier the router keys a node that names none with (run_loop.rs,
node.tier.unwrap_or(Tier::Tier2)), so its evidence is keyed as routing looks it up.")

(defparameter *evidence-floor* 5
  "The kernel's evidence floor, a constant compiled in there. The digest says
which entries clear it; the file carries every entry, and the kernel ignores
those below it with a diagnostic.")

(defparameter *priors-file-limit* (* 1024 1024)
  "The most UTF-8 bytes a proposal may hold; the kernel refuses a larger file.")

(defparameter *max-priors* 1024 "The most entries a v1 file holds.")
(defparameter *max-count* 4096 "The most attempts, or references, one v1 entry holds.")
(defparameter *max-graphs* 4096 "The most graph ids a v1 file's source lists.")
(defparameter *max-model-bytes* 256 "The longest model_ref, in UTF-8 bytes (#134).")
(defparameter *max-harness* 64 "The longest harness name a v1 entry holds.")

(defparameter *prior-lines* 40
  "How many prior keys the digest lists before it counts the rest.")

(defparameter +generator-name+ "nodecode-meshloop")
(defparameter +generator-version+ "0.4.0"
  "This cell's version as the proposal's generator; the .asd's version.")

(defparameter +finished-statuses+ '("completed" "failed" "cancelled")
  "inspect's overall status of a run that is finished: what learn folds when no
graph is named.")

;;; --- the fold ----------------------------------------------------------------------

(defun utf-8-length (text)
  "How many bytes TEXT takes in UTF-8."
  (length (sb-ext:string-to-octets text :external-format :utf-8)))

(defun blank-or-control-p (c)
  "Whether C is whitespace or a control character, as the v1 schema's pattern
for model_ref reads them (\\s and U+0000-U+001F, U+007F-U+009F)."
  (let ((code (char-code c)))
    (or (<= code 32) (<= #x7f code #xa0)
        (member code '(#x1680 #x2028 #x2029 #x202f #x205f #x3000 #xfeff))
        (<= #x2000 code #x200a))))

(defun model-usable-p (model)
  "Whether MODEL is a model_ref a v1 entry may carry: #134's predicate (not
empty, at most 256 bytes, no leading -, no whitespace or control character),
and not `unverified' (ADR 0039)."
  (and (stringp model)
       (plusp (length model))
       (<= (utf-8-length model) *max-model-bytes*)
       (char/= #\- (char model 0))
       (notany #'blank-or-control-p model)
       (string/= model "unverified")))

(defun attempt-exclusion (attempt state)
  "Why ATTEMPT, of a node in STATE, is no evidence, as one of
+EXCLUSION-REASONS+, or NIL when it counts. A missing or null field reads as
not holding."
  (cond ((not (equal (nlk:json-value attempt :string "dispatch_mode") "subprocess")) "not_subprocess")
        ((not (nlk:json-value attempt :boolean "model_passed")) "model_not_passed")
        ((not (model-usable-p (nlk:json-value attempt :string "model_ref"))) "model_unverified")
        ((equal state "Cancelled") "cancelled")
        ((not (member (nlk:json-value attempt :string "check_result") '("passed" "failed") :test #'equal))
         "no_check_result")))

(defun node-tier (node graph)
  "NODE's tier as routing keys it: its tier, or Tier2 when it names none."
  (let ((tier (nlk:json-value node :any "tier")))
    (cond ((null tier) +default-tier+)
          ((member tier +tiers+ :test #'equal) tier)
          (t (fail "node ~a of ~a has tier ~s, which a v1 priors file cannot carry; ~
                    no proposal written" (nlk:json-value node :integer "id") graph
                    (nlk:one-line (princ-to-string tier) :cap 40))))))

(defun counted-harness (attempt graph task)
  "ATTEMPT's harness, which a counted attempt must carry as a v1 entry can."
  (let ((harness (nlk:json-value attempt :string "harness")))
    (unless (and harness (<= 1 (length harness) *max-harness*))
      (fail "attempt ~a of task ~a in ~a has no harness a v1 priors file can carry; no proposal written"
            (nlk:json-value attempt :integer "attempt_id") task graph))
    harness))

(defun id-of (object key what graph)
  "OBJECT's integer KEY, at least 0, or a refusal naming WHAT in GRAPH."
  (let ((id (nlk:json-value object :integer key)))
    (unless (and id (>= id 0))
      (fail "~a in ~a has no ~a meshloop could have written; no proposal written" what graph key))
    id))

(defun key< (a b)
  "Whether key A, (HARNESS MODEL TIER), sorts before B: by code point, field by field."
  (loop for x in a for y in b
        do (cond ((string< x y) (return t))
                 ((string< y x) (return nil)))
        finally (return nil)))

(defun ref< (a b)
  "Whether reference A, (GRAPH TASK ATTEMPT PASSED), sorts before B."
  (destructuring-bind (ga ta aa &rest x) a
    (declare (ignore x))
    (destructuring-bind (gb tb ab &rest y) b
      (declare (ignore y))
      (cond ((string< ga gb) t)
            ((string< gb ga) nil)
            ((/= ta tb) (< ta tb))
            (t (< aa ab))))))

(defun fold-priors (runs)
  "Fold RUNS, a list of (GRAPH-ID . DATA) with DATA the data object of
`meshloop inspect --attempts --graph GRAPH-ID --json', into routing priors.
=> a plist:
  :graphs     the graph ids folded, each once, sorted
  :priors     one (:harness :model-ref :tier :attempts :successes :provenance)
              per key (harness, model_ref, tier), sorted by key; :provenance
              is ((GRAPH TASK ATTEMPT PASSED) ...), sorted, one per counted
              attempt, and :successes counts those with PASSED true
  :excluded   ((REASON . COUNT) ...), every reason of +EXCLUSION-REASONS+ in order
  :counted    attempts counted
  :duplicates attempts seen again under the same (graph, task, attempt), dropped
A graph named twice is folded once, from its first DATA. The answer depends on
the runs' content only, never on their order."
  (let ((graphs (sort (remove-duplicates (mapcar #'car runs) :test #'string= :from-end t) #'string<))
        (seen (make-hash-table :test #'equal))
        (keys (make-hash-table :test #'equal))
        (excluded (mapcar (lambda (reason) (cons reason 0)) +exclusion-reasons+))
        (counted 0) (duplicates 0))
    (when (> (length graphs) *max-graphs*)
      (fail "names ~d graphs; a v1 priors file lists ~d at most. Name fewer with --graph"
            (length graphs) *max-graphs*))
    (dolist (graph graphs)
      (loop with data = (cdr (assoc graph runs :test #'string=))
            for node across (nlk:json-array data "nodes")
            for task = (id-of node "id" "a node" graph)
            for state = (nlk:json-value node :string "state")
            do (loop for attempt across (nlk:json-array node "attempts")
                     for id = (id-of attempt "attempt_id" (format nil "an attempt of task ~a" task) graph)
                     for ref = (list graph task id)
                     do (if (gethash ref seen)
                            (incf duplicates)
                            (let ((reason (attempt-exclusion attempt state)))
                              (setf (gethash ref seen) t)
                              (if reason
                                  (incf (cdr (assoc reason excluded :test #'string=)))
                                  (let ((key (list (counted-harness attempt graph task)
                                                   (nlk:json-value attempt :string "model_ref")
                                                   (node-tier node graph))))
                                    (incf counted)
                                    (push (list graph task id
                                                (equal (nlk:json-value attempt :string "check_result") "passed"))
                                          (gethash key keys)))))))))
    (let ((priors (loop for key being the hash-keys of keys using (hash-value refs)
                        collect (destructuring-bind (harness model tier) key
                                  (list :key key :harness harness :model-ref model :tier tier
                                        :attempts (length refs)
                                        :successes (count-if #'fourth refs)
                                        :provenance (sort (copy-list refs) #'ref<))))))
      (setf priors (sort priors #'key< :key (lambda (entry) (getf entry :key))))
      (when (> (length priors) *max-priors*)
        (fail "folds ~d keys; a v1 priors file holds ~d at most. Name fewer graphs with --graph"
              (length priors) *max-priors*))
      (nlk:when-let (big (find-if (lambda (entry) (> (getf entry :attempts) *max-count*)) priors))
        (fail "~a / ~a / ~a has ~d attempts; a v1 entry holds ~d at most. Name fewer graphs with --graph"
              (getf big :harness) (getf big :model-ref) (getf big :tier) (getf big :attempts) *max-count*))
      (list :graphs graphs :priors priors :excluded excluded
            :counted counted :duplicates duplicates))))

;;; --- the file ----------------------------------------------------------------------
;;; Written by hand, keys in the schema's order, so the same fold is the same
;;; bytes: no hash-table order, no float, no locale.

(defun json-quoted (text)
  "TEXT as a JSON string literal: \\\" and \\\\ escaped, control characters as \\uXXXX,
everything else as it is (the file is UTF-8)."
  (with-output-to-string (out)
    (write-char #\" out)
    (loop for c across text
          for code = (char-code c)
          do (cond ((char= c #\") (write-string "\\\"" out))
                   ((char= c #\\) (write-string "\\\\" out))
                   ((or (< code 32) (= code 127)) (format out "\\u~4,'0x" code))
                   (t (write-char c out))))
    (write-char #\" out)))

(defun iso-time (universal-time)
  "UNIVERSAL-TIME as RFC 3339 in UTC: 2026-11-11T09:30:00Z."
  (multiple-value-bind (s mi h d mo y) (decode-universal-time universal-time 0)
    (format nil "~4,'0d-~2,'0d-~2,'0dT~2,'0d:~2,'0d:~2,'0dZ" y mo d h mi s)))

(defun priors-json (fold generated-at)
  "The v1 proposal for FOLD (FOLD-PRIORS), generated at GENERATED-AT, an RFC 3339
string, as the text of the file, ending in a newline."
  (with-output-to-string (out)
    (format out "{~%  \"schema\": \"meshloop/advisory-priors\",~%  \"version\": 1,~%")
    (format out "  \"generated_at\": ~a,~%" (json-quoted generated-at))
    (format out "  \"generator\": {\"name\": ~a, \"version\": ~a},~%"
            (json-quoted +generator-name+) (json-quoted +generator-version+))
    (format out "  \"source\": {\"graph_ids\": [~{~a~^, ~}]},~%"
            (mapcar #'json-quoted (getf fold :graphs)))
    (let ((priors (getf fold :priors)))
      (if (null priors)
          (format out "  \"priors\": []~%")
          (progn
            (format out "  \"priors\": [~%")
            (loop for (entry . more) on priors
                  do (format out "    {~%      \"harness\": ~a,~%      \"model_ref\": ~a,~%      \"tier\": ~a,~%      ~
                                  \"dispatch_mode\": \"subprocess\",~%      \"attempts\": ~d,~%      \"successes\": ~d,~%      ~
                                  \"provenance\": [~%"
                             (json-quoted (getf entry :harness)) (json-quoted (getf entry :model-ref))
                             (json-quoted (getf entry :tier)) (getf entry :attempts) (getf entry :successes))
                     (loop for (ref . rest) on (getf entry :provenance)
                           do (destructuring-bind (graph task attempt passed) ref
                                (format out "        {\"graph_id\": ~a, \"task_id\": ~d, \"attempt_id\": ~d, \"passed\": ~:[false~;true~]}~:[~;,~]~%"
                                        (json-quoted graph) task attempt passed rest)))
                     (format out "      ]~%    }~:[~;,~]~%" more))
            (format out "  ]~%"))))
    (format out "}~%")))

;;; --- the digest --------------------------------------------------------------------

(defun shown (text)
  "TEXT fit to show: control and Unicode format (Cf, bidi among them)
characters dropped, one line, clipped."
  (nlk:one-line (remove-if (lambda (c)
                             (or (< (char-code c) 32) (<= #x7f (char-code c) #x9f)
                                 (eq (sb-unicode:general-category c) :cf)))
                           (princ-to-string text))
                :cap *text-cap*))

(defun shell-word (text)
  "TEXT as one shell word: as it is when it holds only safe characters, else in
single quotes."
  (if (and (plusp (length text))
           (every (lambda (c) (or (and (< (char-code c) 128) (alphanumericp c)) (find c "._/@:+=-"))) text))
      text
      (with-output-to-string (out)
        (write-char #\' out)
        (loop for c across text
              do (if (char= c #\') (write-string "'\\''" out) (write-char c out)))
        (write-char #\' out))))

(defun graph-list (graphs)
  "GRAPHS for a line: the first twelve, then how many more."
  (format nil "~{~a~^, ~}~@[ and ~d more~]"
          (subseq graphs 0 (min 12 (length graphs)))
          (and (> (length graphs) 12) (- (length graphs) 12))))

(defun digest-lines (fold)
  "The lines of the digest that FOLD alone gives: one a prior key, then the
exclusions by reason."
  (with-output-to-string (out)
    (loop for entry in (getf fold :priors)
          for index from 0
          while (< index *prior-lines*)
          do (let ((attempts (getf entry :attempts)) (successes (getf entry :successes)))
               (format out "~%  prior ~a / ~a / ~a: ~d attempt~:p, ~d passed, ~d failed; ~:[below the kernel's floor (n >= ~d): it will be ignored~;~*clears the kernel's floor (n >= ~d)~]"
                       (shown (getf entry :harness)) (shown (getf entry :model-ref)) (getf entry :tier)
                       attempts successes (- attempts successes)
                       (>= attempts *evidence-floor*) *evidence-floor* *evidence-floor*)))
    (when (> (length (getf fold :priors)) *prior-lines*)
      (format out "~%  ... ~d more prior keys in the file" (- (length (getf fold :priors)) *prior-lines*)))
    (format out "~%  excluded by reason: ~{~a~^, ~}"
            (loop for (reason . count) in (getf fold :excluded) collect (format nil "~a ~d" reason count)))
    (when (plusp (getf fold :duplicates))
      (format out "~%  ~d attempt row~:p seen twice under one (graph, task, attempt) dropped" (getf fold :duplicates)))))

(defun promote-lines (path)
  "What the operator types to promote the proposal at PATH; never run here."
  (let ((file (shell-word path)))
    (format nil "~%  This cell never promotes. To promote, type these in a shell yourself, reading the ~
                 first one's diagnostics before the second:~%    meshloop priors check --file ~a~%    ~
                 meshloop priors accept --file ~a --as <your-name>" file file)))

(defun fold-digest (fold path digest skipped promoted)
  "The answer of /meshloop learn: PATH and DIGEST of the proposal written (NIL
when none was), each prior key with its counts, exclusions by reason, the
graphs SKIPPED ((GRAPH . WHY) ...), PROMOTED (a line on the priors in effect)
and the commands that promote."
  (with-output-to-string (out)
    (if path
        (format out "meshloop learn: wrote a routing-prior proposal to ~a (~a); folded ~d graph~:p (~a): ~
                     ~d attempt~:p counted, ~d excluded"
                path digest (length (getf fold :graphs)) (graph-list (getf fold :graphs))
                (getf fold :counted) (reduce #'+ (getf fold :excluded) :key #'cdr))
        (format out "meshloop learn: no attempt counts as evidence, so no proposal was written; folded ~d graph~:p~
                     ~:[~;~:* (~a)~]: ~d excluded"
                (length (getf fold :graphs)) (and (getf fold :graphs) (graph-list (getf fold :graphs)))
                (reduce #'+ (getf fold :excluded) :key #'cdr)))
    (write-string (digest-lines fold) out)
    (loop for (graph . why) in skipped
          do (format out "~%  skipped ~a: ~a" graph why))
    (when promoted (format out "~%  ~a" promoted))
    (when path (write-string (promote-lines path) out))))

;;; --- reading meshloop --------------------------------------------------------------

(defun priors-attempts (graph session-id &key tolerant)
  "The data object of `inspect --attempts --graph GRAPH', checked against the
contract's inspect_attempts and priors_attempts fields: a binary older than
#139 lacks the latter, and learn then folds nothing. => (values DATA REFUSAL):
with TOLERANT, a graph meshloop refuses to inspect answers NIL and meshloop's
refusal instead of failing; a missing field fails either way."
  (let ((object (ask "inspect" (list "--attempts" "--graph" graph) session-id)))
    (if (and tolerant (not (nlk:json-value object :boolean "ok")))
        (values nil (refusal-of object))
        (let ((object (checked (ok-or-refuse object) "inspect_attempts")))
          (values (nlk:json-value (checked object "priors_attempts") :object "data") nil)))))

(defun discovered-graphs (session-id)
  "The graphs learn folds when none is named: the newest run and every graph
`workspace inventory' still holds an entry for, each once, sorted. meshloop
has no verb listing every graph in its store, so a graph whose worktrees were
all reclaimed is only folded when named."
  (let ((found '()))
    (let* ((status (ok-or-refuse (ask "status" '() session-id)))
           (data (nlk:json-value status :object "data")))
      (when (and data (nth-value 1 (gethash "graph_id" data)))
        (push (nlk:json-value (checked status "status") :string "data" "graph_id") found)))
    (let ((inventory (handler-case (checked (ok-or-refuse (ask "workspace" (list "inventory") session-id
                                                               :command "meshloop:workspace-inventory"))
                                            "workspace_inventory")
                       (meshloop-error () nil))))
      (loop for entry across (nlk:json-array inventory "data" "entries")
            for graph = (nlk:json-value entry :string "graph_id")
            when graph do (push graph found)))
    (sort (remove-duplicates (remove-if-not #'graph-id-p found) :test #'string=) #'string<)))

(defun promoted-line (session-id)
  "One line on the priors in effect, from `meshloop priors show', or why it is
unknown. Never a refusal: the proposal stands without it."
  (handler-case
      (let* ((object (checked (ok-or-refuse (ask "priors" (list "show") session-id :command "meshloop:priors-show"))
                              "priors_show"))
             (data (nlk:json-value object :object "data")))
        (if (nlk:json-value data :boolean "in_effect")
            (format nil "promoted priors in effect: ~a~@[; changed_top on ~d~]~@[ of ~d routed attempts~]"
                    (let ((sha (nlk:json-value data :string "priors_sha256"))) (if sha (shown sha) "?"))
                    (nlk:json-value data :integer "routing" "changed_top")
                    (nlk:json-value data :integer "routing" "attempts"))
            "promoted priors in effect: none"))
    (meshloop-error (condition)
      (format nil "promoted priors in effect: unknown (meshloop priors show: ~a)"
              (nlk:one-line (meshloop-error-detail condition) :cap *text-cap*)))))

;;; --- where the proposal goes --------------------------------------------------------

(defun stamp (universal-time)
  "UNIVERSAL-TIME in UTC as 20261111T093000Z, for a file name."
  (multiple-value-bind (s mi h d mo y) (decode-universal-time universal-time 0)
    (format nil "~4,'0d~2,'0d~2,'0dT~2,'0d~2,'0d~2,'0dZ" y mo d h mi s)))

(defun default-proposal-path (root universal-time)
  "Where a proposal goes when the operator names none: this cell's own state
under the nodecode home, a folder per repository."
  (nlk:home (format nil "meshloop/priors/~a/proposal-~a.json"
                    (board-name-of (real-root root)) (stamp universal-time))))

(defparameter +out-usage+
  "--out takes a new .json file inside this repository's .agent-runs/, such as .agent-runs/priors/proposal.json")

(defun out-file (root text)
  "TEXT, the path typed after --out, read without touching the disk: ending in
.json, with no .. step, and inside ROOT's .agent-runs/ as written.
=> (values FILE RUNS), FILE's native path and the real .agent-runs/ folder's."
  (let ((usage +out-usage+))
    (unless (and (<= 1 (length text) 1024) (char/= #\- (char text 0))
                 (notany (lambda (c) (< (char-code c) 32)) text)
                 (suffix-p ".json" text))
      (fail "~s is no proposal path: ~a" (nlk:one-line text :cap 40) usage))
    (when (find ".." (uiop:split-string text :separator "/\\") :test #'string=)
      (fail "~s has a .. step: ~a" (nlk:one-line text :cap 40) usage))
    (let* ((real (uiop:ensure-directory-pathname (real-root root)))
           (runs (uiop:native-namestring (merge-pathnames ".agent-runs/" real)))
           (file (uiop:native-namestring
                  (if (uiop:absolute-pathname-p (uiop:parse-native-namestring text))
                      (uiop:parse-native-namestring text)
                      (merge-pathnames (uiop:parse-native-namestring text)
                                       (uiop:ensure-directory-pathname root))))))
      ;; The root as the session names it and as links resolve it both count.
      (let ((typed-runs (uiop:native-namestring
                         (merge-pathnames ".agent-runs/" (uiop:ensure-directory-pathname root)))))
        (unless (or (and (> (length file) (length runs)) (string= runs file :end2 (length runs)))
                    (and (> (length file) (length typed-runs)) (string= typed-runs file :end2 (length typed-runs))))
          (fail "~a is outside ~a: ~a" file runs usage))
        (values file runs real)))))

(defun agent-runs-path (root text)
  "TEXT, the path typed after --out, as a file inside ROOT's .agent-runs/:
OUT-FILE's rule, not there yet, and inside .agent-runs/ once links are
resolved; its folder is made only then. A refusal otherwise."
  (let ((usage +out-usage+))
    (multiple-value-bind (file runs real) (out-file root text)
      (when (probe-file file)
        (fail "~a exists; name a new file" file))
      ;; Before making a folder: the nearest one that exists must be the
      ;; repository itself or inside its .agent-runs/, links resolved.
      (let ((existing (loop for dir = (uiop:pathname-directory-pathname file)
                              then (uiop:pathname-parent-directory-pathname dir)
                            for true = (ignore-errors (truename dir))
                            when true return (uiop:native-namestring true)
                            until (equal dir (uiop:pathname-parent-directory-pathname dir)))))
        (unless (and existing
                     (or (string= existing (uiop:native-namestring real))
                         (and (>= (length existing) (length runs))
                              (string= runs existing :end2 (length runs)))))
          (fail "~a resolves outside ~a through a link: ~a" file runs usage)))
      (ensure-directories-exist file :mode #o700)
      (let ((parent (ignore-errors (truename (uiop:pathname-directory-pathname file)))))
        (unless (and parent
                     (let ((resolved (uiop:native-namestring parent)))
                       (and (>= (length resolved) (length runs))
                            (string= runs resolved :end2 (length runs)))))
          (fail "~a resolves outside ~a through a link: ~a" file runs usage)))
      file)))

;;; --- the commands ------------------------------------------------------------------

(defparameter +learn-usage+ "/meshloop learn [--graph G ...] [--out FILE]")

(defun learn-words (text)
  "TEXT's graphs and --out. => (values GRAPHS OUT): every --graph G and bare
word a graph id, in the order typed; OUT the path after --out, or NIL."
  (let ((words (nlk:split-words (or text ""))) (graphs '()) (out nil))
    (loop while words
          do (let ((word (pop words)))
               (cond ((string= word "--graph")
                      (let ((graph (pop words)))
                        (unless graph (fail "--graph needs a graph id: ~a" +learn-usage+))
                        (push (graph-of graph) graphs)))
                     ((string= word "--out")
                      (when out (fail "--out is given twice"))
                      (setf out (or (pop words) (fail "--out needs a file: ~a" +learn-usage+))))
                     ((and (> (length word) 1) (char= #\- (char word 0)))
                      (fail "takes no ~a: ~a" (nlk:one-line word :cap 40) +learn-usage+))
                     (t (push (graph-of word) graphs)))))
    (values (nreverse graphs) out)))

(defun learn-of (session-id text)
  "/meshloop learn: fold the named graphs (or every finished one meshloop
still lists) into a routing-prior proposal, write it, and answer the digest."
  (answering "learn"
             (lambda ()
               (multiple-value-bind (named out) (learn-words text)
                 (let* ((root (session-directory session-id))
                        (now (progn (when out (out-file root out)) (get-universal-time)))
                        (skipped '())
                        (runs (if named
                                  (loop for graph in (remove-duplicates named :test #'string= :from-end t)
                                        collect (cons graph (priors-attempts graph session-id)))
                                  (loop for graph in (discovered-graphs session-id)
                                        for (data refusal) = (multiple-value-list
                                                              (priors-attempts graph session-id :tolerant t))
                                        for status = (and data (nlk:json-value data :string "status"))
                                        if (member status +finished-statuses+ :test #'equal)
                                          collect (cons graph data)
                                        else
                                          do (push (cons graph (if refusal
                                                                   (format nil "meshloop refused to inspect it: ~a" refusal)
                                                                   (format nil "~a, not finished; name it with --graph to fold it anyway"
                                                                           (or status "?"))))
                                                   skipped)))))
                   (when (and (null runs) (null named))
                     (fail "found no finished run to fold~:[~;~:* (~{~a~^; ~})~]. Name graphs with --graph: ~a"
                           (mapcar (lambda (s) (format nil "~a: ~a" (car s) (cdr s))) (reverse skipped))
                           +learn-usage+))
                   (let* ((fold (fold-priors runs))
                          (promoted (promoted-line session-id)))
                     (if (null (getf fold :priors))
                         (fold-digest fold nil nil (reverse skipped) promoted)
                         (let* ((text (priors-json fold (iso-time now)))
                                (bytes (utf-8-length text)))
                           (when (> bytes *priors-file-limit*)
                             (fail "the proposal would hold ~d bytes; the kernel reads ~d at most. ~
                                    Name fewer graphs with --graph" bytes *priors-file-limit*))
                           (let ((path (if out (agent-runs-path root out) (default-proposal-path root now))))
                             (nlk:write-file-atomically path text :mode #o600 :directory-mode #o700)
                             (fold-digest fold (uiop:native-namestring path) (nlk:sha256-text text)
                                          (reverse skipped) promoted))))))))))

(defun priors-file-of (text usage)
  "TEXT as a file path meshloop reads, relative to the repository root, or a refusal."
  (unless (and (stringp text) (<= 1 (length text) 1024) (char/= #\- (char text 0))
               (notany (lambda (c) (< (char-code c) 32)) text))
    (fail "needs a file: ~a" usage))
  text)

(defun priors-of (session-id text)
  "/meshloop priors show|check: meshloop's read-only priors verbs, summarized.
priors accept and revoke are a person's to type in a shell; the cell runs
neither and says so."
  (answering "priors"
             (lambda ()
               (let* ((words (nlk:split-words (or text "")))
                      (sub (string-downcase (or (first words) ""))))
                 (cond
                   ((string= sub "show")
                    (when (rest words) (fail "show takes nothing more: /meshloop priors show"))
                    (format nil "meshloop priors: ~a" (promoted-line session-id)))
                   ((string= sub "check")
                    (let* ((usage "/meshloop priors check FILE")
                           (rest (rest words))
                           (file (priors-file-of (if (equal (first rest) "--file") (second rest) (first rest)) usage)))
                      (when (> (length rest) (if (equal (first rest) "--file") 2 1))
                        (fail "takes one file: ~a" usage))
                      (summarize-priors-check
                       file (checked (ok-or-refuse (ask "priors" (list "check" "--file" file) session-id
                                                        :command "meshloop:priors-check"))
                                     "priors_check"))))
                   ((member sub '("accept" "revoke") :test #'string=)
                    (fail "refused: this cell never ~:[revokes~;promotes~] priors (ADR 0038). A person types it ~
                           in a shell, naming themselves: meshloop priors ~a ~:[~;--file FILE ~]--as <your-name>"
                          (string= sub "accept") sub (string= sub "accept")))
                   (t (fail "takes show or check FILE: /meshloop priors show | /meshloop priors check FILE")))))))

(defun summarize-priors-check (file object)
  "`meshloop priors check --file FILE --json' as a few lines: what the kernel
would use and every diagnostic code, then how a person promotes it."
  (let* ((data (nlk:json-value object :object "data"))
         (diagnostics (nlk:json-array data "diagnostics")))
    (with-output-to-string (out)
      (format out "meshloop priors check: ~a: ~:[~;~:*file fault ~a, no priors would apply; ~]~
                   ~a usable, ~a ignored; ~d diagnostic~:p (read-only, nothing promoted)"
              file (let ((fault (nlk:json-value data :string "file_fault"))) (and fault (shown fault)))
              (or (nlk:json-value data :integer "usable") "?") (or (nlk:json-value data :integer "ignored") "?")
              (length diagnostics))
      (loop for d across diagnostics
            for index from 0
            while (< index *prior-lines*)
            do (format out "~%  ~a~@[: ~a~]" (shown (nlk:json-value d :string "code"))
                       (let ((detail (nlk:json-value d :string "detail"))) (and detail (shown detail)))))
      (when (> (length diagnostics) *prior-lines*)
        (format out "~%  ... ~d more" (- (length diagnostics) *prior-lines*)))
      (format out "~%  To promote it, a person types in a shell: meshloop priors accept --file ~a --as <your-name>"
              (shell-word file)))))
