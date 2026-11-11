;;;; nodecode-meshloop.asd --- drive meshloop runs from a nodecode session.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; Optional cell, not part of meshloop's build: nothing in the Rust
;;;; workspace loads it. The nodecode folder loader loads it at boot when this
;;;; directory is linked under <nodecode home>/cells/nodecode-meshloop; the
;;;; gateway calls START-CELL, which reads the `meshloop' section, reads
;;;; contract.json, registers /meshloop, installs the :tool veto and puts the
;;;; manual on (help :meshloop). board.lisp projects runs onto a nodecode
;;;; Board when meshloop.board is on; the nodecode-board cell is an optional
;;;; peer found at run time, not a dependency. See README.md.

(defsystem "nodecode-meshloop"
  :description "meshloop doctor, status, inspect and report, the operator's plan, review-plan, run, resume, cancel, accept and integrate, an optional one-way Board projection, and a routing-prior proposal it never promotes, through the meshloop CLI's --json output"
  :license "Apache-2.0"
  :version "0.4.0"
  :depends-on ("nodecode")
  :serial t
  :components ((:file "package")
               (:file "cell")
               (:file "gates")
               (:file "board")
               (:file "learn")
               (:file "drive")
               (:static-file "contract.json")))
