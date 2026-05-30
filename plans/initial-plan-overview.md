# Project Overview

## What this project is

This project is a **modern, cross-platform amateur radio operating platform**, built in Rust, that integrates:

* real-time station control
* logging (contest and non-contest)
* DX intelligence (spots, station data, propagation)
* award tracking and confirmation workflows
* operator-centric workflows (SO1R/SO2R, ESM, etc.)

into a **single coherent application**.

It is not a “logger with features added.”
It is an **operating system for the station**.

---

## What this project is *not*

* Not a DXKeeper clone
* Not a DXLabSuite rewrite
* Not just a contest logger
* Not just a logbook library

Those are all **inputs and references**, not targets.

---

## The core idea

The fundamental shift is:

> **From “logbook-centered software” → “operating-centered software”**

In legacy systems:

* The logbook is the center
* Everything feeds into or hangs off QSOs

In this system:

* The **operating session** is the center
* QSOs are one type of durable outcome of operating

---

## Mental model

### The system models a *live operating environment*

At any moment, the system understands:

* who is operating
* from where (station/QTH)
* on which radio(s)
* on what frequency/mode
* what stations are active (spots, decoded signals)
* what is needed (awards, multipliers, goals)
* what has already been worked/logged
* what actions are in progress (sync, QSL, etc.)

Logging a QSO is just one transition in that system.

---

## Core domains

The application is composed of cooperating domains, not separate apps.

### 1. Operating domain

Real-time context:

* rig state (via `riglib`)
* SO2R/device control (`otrsp`)
* CW/digital interfaces (`winkey`, etc.)
* active band/mode/frequency
* operator session
* spot interaction (`dxfeed`)

This is the *live brain* of the station.

---

### 2. Logbook domain

Durable record of activity:

* canonical QSOs
* exchange data
* editing/history
* import/export (ADIF, DXKeeper migration)

This is **not the center**, but it is critical.

---

### 3. Station-data domain

Understanding the world:

* callsign → entity resolution
* DXCC, zones, grids, IOTA, etc.
* history/overrides
* super check partial

This feeds *everything else*.

---

### 4. Awards domain

Interprets QSOs against goals:

* DXCC, WAS, WPX, etc.
* band/mode tracking
* verification/credit logic
* imported credits and reconciliation

This answers:

> “Did this QSO matter?”

---

### 5. Confirmation / sync domain

External system integration:

* LoTW
* eQSL
* Club Log
* QRZ

Handles:

* upload/download
* confirmation states
* retries and reconciliation

---

### 6. QSL domain

Paper workflow:

* outgoing QSL queues
* routing (direct/bureau/etc.)
* printing/labeling
* tracking

---

### 7. Persistence domain

Local system of record:

* SQLite
* canonical facts
* workflow queues
* materialized views (award progress, neededness)

---

### 8. UI / application shell (iced 0.14)

Operator interface:

* keyboard-first workflows
* single-window, panel-based UI
* reactive state → view model
* reducer + effects architecture

The UI reflects the system—it does not define it.

---

## Architectural philosophy

### 1. Domain-driven, not UI-driven

The UI consumes domain state and emits commands.
It does not contain business logic.

---

### 2. Event-driven internally

The system reacts to events like:

* QSO created
* rig frequency changed
* spot received
* confirmation received
* award state updated

This keeps domains loosely coupled.

---

### 3. Separation of concerns (critical)

You must strictly separate:

#### Canonical facts

* QSOs
* station locations
* operators

#### Derived interpretations

* neededness
* award progress
* validation flags

#### Workflow state

* sync queues
* QSL queues
* pending jobs

This is where legacy systems blur boundaries. You should not.

---

### 4. Local-first

The system must function fully offline:

* logging
* operating
* award tracking
* station resolution

External services are **enhancements**, not dependencies.

---

### 5. Strong typing

Everything meaningful is typed:

* Band
* Mode
* Submode
* Propagation
* Confirmation state
* Award units
* Station identity

This replaces the string-heavy legacy ecosystem.

---

### 6. Extensibility without schema explosion

Instead of:

* 150 QSO columns

Use:

* core QSO
* structured extensions (exchange fields, metadata, service state)

---

## Key differentiators vs legacy systems

### 1. Unified system instead of multiple apps

DXLabSuite = loosely coupled tools
This project = **coherent platform**

---

### 2. Clean domain model

Legacy:

* everything flattened into QSOs

Modern:

* multiple aggregates
* explicit relationships

---

### 3. Explicit workflow modeling

Legacy:

* state scattered in flags

Modern:

* explicit queues, jobs, transitions

---

### 4. Provenance and auditability

You can answer:

* where did this data come from?
* was it user-entered or inferred?
* what changed and why?

---

### 5. Performance-conscious architecture

* async IO (`riglib`, feeds)
* UI remains responsive
* derived state computed incrementally

---

### 6. Designed for SO2R and modern operating from the start

Not retrofitted.

---

## Guiding principle

Every decision should answer:

> “Am I preserving the *capability*, or copying the *implementation*?”

Always preserve capability.
Never copy implementation unless there is a compelling reason.

---

## Short version (for your repo README)

If you want a concise version:

> This project is a modern, cross-platform amateur radio operating platform built in Rust.
> It integrates station control, logging, DX intelligence, awards, and confirmation workflows into a single, coherent system.
>
> Unlike legacy tools that are centered around logbooks or split into multiple applications, this system models the entire operating environment: live station state, operator context, and workflows.
>
> It is designed to support advanced operating scenarios (including SO2R), maintain full offline capability, and provide a strongly-typed, event-driven architecture that cleanly separates canonical data, derived state, and workflow processes.

---

## Final note

If you keep this framing in mind, everything you’ve been deciding—dropping `qsolog` as the center, splitting persistence, using domain crates, adopting iced with a reducer model—stays aligned.

If you drift away from this framing, you will slowly rebuild DXKeeper in Rust.

---

If you want, the next step I’d recommend is turning this into a **`architecture.md`** file you can commit to your repo and use to anchor future decisions.

