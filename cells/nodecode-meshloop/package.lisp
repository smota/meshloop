;;;; package.lisp --- the MESHLOOP package: four read-only verbs and the condition.
;;;;
;;;; SPDX-License-Identifier: Apache-2.0
;;;;
;;;; One package, nicknamed MESHLOOP so an eval form reads (meshloop:status).
;;;; Every exported verb answers a STRING and never signals: a refusal is one
;;;; line that starts with "meshloop <verb>:". Only read-only verbs are
;;;; exported; the commands that write are /meshloop slash commands the
;;;; operator types (drive.lisp), never eval verbs (ADR 0038). The settings
;;;; derive from the one `meshloop' section declared in drive.lisp
;;;; (NLE:DEFINE-CELL), read with (SETTING :binary).

(defpackage #:nodecode-meshloop
  (:use #:cl)
  ;; CL:INSPECT is the standard's; this package's INSPECT is the verb.
  (:shadow #:inspect)
  (:nicknames #:meshloop)
  (:export
   ;; verbs, every one a string
   #:doctor #:status #:inspect #:report
   ;; the one condition
   #:meshloop-error))

(in-package #:nodecode-meshloop)

(nlk:define-peripheral meshloop :not-running t)
