;;;; nodecode-meshloop.asd --- read-only meshloop views for a nodecode session.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; Optional cell, not part of meshloop's build: nothing in the Rust
;;;; workspace loads it. The nodecode folder loader loads it at boot when this
;;;; directory is linked under <nodecode home>/cells/nodecode-meshloop; the
;;;; gateway calls START-CELL, which reads the `meshloop' section, reads
;;;; contract.json, registers /meshloop and puts the manual on
;;;; (help :meshloop). See README.md.

(defsystem "nodecode-meshloop"
  :description "Read-only meshloop doctor, status and inspect through the meshloop CLI's --json output"
  :license "Apache-2.0"
  :version "0.1.0"
  :depends-on ("nodecode")
  :serial t
  :components ((:file "package")
               (:file "cell")
               (:static-file "contract.json")))
