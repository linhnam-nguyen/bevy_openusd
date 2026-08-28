# M18-C2 Project operation admission audit

This audit records the admission and duplicate-submit policy for every
asynchronous or worker-backed Project path. The policy is an audit of the
existing M10-M17 ownership boundaries; it does not add a new product
operation.

| Operation | Capacity | Replaceable? | Full behavior |
| --- | ---: | --- | --- |
| Scene inspection | 2 (one worker plus one pending slot) | yes before publish | Keep the newest pending request; the superseded request receives a typed `ConcurrentChange` failure. |
| Model preparation | 4 worker jobs | yes before confirmation | The bounded channel uses `try_send`; a full channel returns typed `Busy`. A prepared artifact is retained only for its `(operation_id, generation)` and is evicted after the bounded retention window. |
| Project publish mutation | 1 per Project | no | `ProjectPublicationCoordinator` resolves one mutex per `ProjectId`; duplicate publication attempts for the same Project serialize, while different Projects remain independent. Stage-outbox capacity is checked before canonical publication. |
| Branch switch | 1 per Project | no | The current production/backend capability is intentionally unavailable, so no asynchronous backend submit path can duplicate it. The fixture-only controller mutation is synchronous and its existing dirty-state guard rejects unsafe switches before mutation. |
| Stage activation | existing renderer/session policy | generation guarded | `SessionCoordinator` remains the single owner of queued activation. Requests carry Project identity, root and generation; acknowledgement requires all command identity fields to match and reuses the existing owner. |

## Path-by-path evidence

### Project read refresh

`AuthoritativeProjectReads` owns one generation for the refresh family. List,
tree and repository replies are applied only when their Project identity and
generation match the current request. A stale reply is discarded and cannot
roll back a newer authoritative value. A failed refresh changes the read
state to unavailable while the retained value is only kept for the explicit
in-flight refresh behavior; it is not projected as current after failure.

### Scene inspection

`ProjectSceneInspectionQueue` has one worker and one replaceable pending job.
Submitting a newer request replaces only the pending job; a job already being
inspected is allowed to finish, and its generation guard prevents stale
publication from becoming current. The replaced pending request is completed
with `ProjectWriteError::ConcurrentChange`, so callers do not wait forever.

### Model preparation

`ProjectModelPreparationQueue` uses a synchronous channel of capacity four and
`try_send`, making admission non-blocking at the boundary. A full queue
returns `ProjectWriteErrorCode::Busy`. The worker stores only the prepared
artifact required for a matching operation and generation; the publication
step consumes that exact key. UI generation ownership and the C1 stale guards
provide logical supersession before confirmation, while the backend channel
provides physical boundedness and never treats a stale completion as current.

### Project publication and stage handoff

`ProjectPublicationCoordinator` is shared by the host and keyed by
`ProjectId`, so one Project cannot publish two canonical mutations at once.
The publication path checks the private stage-mutation outbox before changing
the manifest, then submits the typed handoff after publication. The durable
stage outbox has an explicit capacity of 128 pending records and returns
`Busy` when full. The active-stage owner remains the only code that applies
those records to the LiveStage.

### Branch switch

The backend has no branch-switch command in this phase. The frontend therefore
does not enqueue a production branch operation: backend-authoritative branch
switch requests return the explicit unavailable result, and only the fixture
controller performs the synchronous mutation. This preserves the one-at-a-
time policy without inventing a second backend owner before branch switching
is authorized.

### Stage activation

Stage activation is owned by the existing viewport session coordinator rather
than by Projects. Its pending queue is intentionally governed by the existing
renderer/session policy. M18-C1 makes completion require protocol version,
request ID, Project ID, root and generation equality, so a late activation
acknowledgement is discarded without changing the active context.

### Import progress

`ProjectImportProgressStore` coalesces by `(operation_id, generation)` and
retains at most 64 keys. It is status observation, not a work queue; newer
phases replace older phases for the same operation and the oldest keys are
evicted. UI consumers use the C1 generation guard before presenting or acting
on a completion.

### Catalogue and registry reads

Registry/catalogue reads are synchronous, linear in the number of registered
Projects, and do not submit worker work. An unavailable repository remains a
registered entry and is represented as an unavailable catalogue item; reads
never remove or retarget it.

## Admission conclusions

The physical queues are bounded where worker work or durable handoff can
accumulate. Replaceable work is replaced only before publication, and all
completion paths retain stable identity plus generation checks. The audit
found no second Project publication owner, no duplicate stage activation
owner, and no unbounded Project-specific worker queue to add in this
checkpoint.
