use super::FrameSampleId;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum AnimationDebugFault {
    #[default]
    None,
    FreezeTransformEvidence,
}

impl AnimationDebugFault {
    pub(crate) fn parse(value: Option<&str>) -> Result<Self, String> {
        match value {
            None => Ok(Self::None),
            Some("freeze-transform-evidence") | Some("freeze_transform_evidence") => {
                Ok(Self::FreezeTransformEvidence)
            }
            Some(value) => Err(format!(
                "unsupported animation debug fault `{value}`; available fault: `freeze-transform-evidence`"
            )),
        }
    }

    pub(crate) fn report_name(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::FreezeTransformEvidence => Some("freeze_transform_evidence"),
        }
    }
}

pub(crate) fn apply_transform_evidence(
    fault: AnimationDebugFault,
    sample: FrameSampleId,
    observed: Option<u64>,
    t0: &mut Option<u64>,
) -> Option<u64> {
    if fault != AnimationDebugFault::FreezeTransformEvidence {
        return observed;
    }
    match sample {
        FrameSampleId::T0 => {
            *t0 = observed;
            observed
        }
        FrameSampleId::T1 => (*t0).or(observed),
        _ => observed,
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimationDebugFault, apply_transform_evidence};
    use crate::viewport::transport::frame_signature::FrameSampleId;

    #[test]
    fn freeze_fault_reuses_t0_only_for_t1() {
        let mut t0 = None;
        assert_eq!(
            apply_transform_evidence(
                AnimationDebugFault::FreezeTransformEvidence,
                FrameSampleId::T0,
                Some(11),
                &mut t0,
            ),
            Some(11)
        );
        assert_eq!(
            apply_transform_evidence(
                AnimationDebugFault::FreezeTransformEvidence,
                FrameSampleId::T1,
                Some(22),
                &mut t0,
            ),
            Some(11)
        );
        assert_eq!(
            apply_transform_evidence(
                AnimationDebugFault::FreezeTransformEvidence,
                FrameSampleId::T0RoundTrip,
                Some(11),
                &mut t0,
            ),
            Some(11)
        );
    }
}
