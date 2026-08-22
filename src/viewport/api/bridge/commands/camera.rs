use viewport_protocol::{CameraSource, ViewportEvent, ViewportEventEnvelope};

use crate::viewport::api::ViewportEventOutbox;
use crate::viewport::camera::CameraMount;

pub(super) fn set_camera_source(
    request_id: String,
    source: CameraSource,
    outbox: &mut ViewportEventOutbox,
    camera_mount: &mut CameraMount,
) {
    *camera_mount = match &source {
        CameraSource::Arcball => CameraMount::Arcball,
        CameraSource::Authored { prim_path } => CameraMount::Mounted {
            prim_path: prim_path.clone(),
        },
    };
    outbox.push(ViewportEventEnvelope::new(
        Some(request_id),
        ViewportEvent::CameraSourceChanged { source },
    ));
}
