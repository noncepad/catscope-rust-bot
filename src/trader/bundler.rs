/*
* "landed_tips_25th_percentile": 10000,
    "landed_tips_50th_percentile": 21000,
    "landed_tips_75th_percentile": 77952,
    "landed_tips_95th_percentile": 1000000,
    "landed_tips_99th_percentile": 4759970
*/

use crate::{err::CatscopeGuestError, graph::Lamports};

pub enum BundlerUpdate {
    Astralane(AstralaneInfo),
}

/// Give updates on tips and other relevant information
pub enum AstralaneInfo {
    Tips(AstralaneTips),
}

/// This information should be:
///
///    "landed_tips_25th_percentile": 10000,
///    "landed_tips_50th_percentile": 21000,
///    "landed_tips_75th_percentile": 77952,
///    "landed_tips_95th_percentile": 1000000,
///    "landed_tips_99th_percentile": 4759970
pub struct AstralaneTips {
    pub landed_tips: [Lamports; 5],
}
impl AstralaneTips {
    /// Given a desired chance of landing a transaction, get the requried tip.
    pub fn landing(&self, desired_probability: LandingProbability) -> Lamports {
        match desired_probability {
            LandingProbability::VeryLow => self.landed_tips[0],
            LandingProbability::Low => self.landed_tips[1],
            LandingProbability::Medium => self.landed_tips[2],
            LandingProbability::High => self.landed_tips[3],
            LandingProbability::VeryHigh => self.landed_tips[4],
        }
    }
}

pub trait BundlerTipCalculator {
    fn landing(&self, _desired_probability: LandingProbability) -> Lamports {
        0
    }
}

pub enum LandingProbability {
    VeryLow,
    Low,
    Medium,
    High,
    VeryHigh,
}

impl TryFrom<&[u8]> for BundlerUpdate {
    type Error = CatscopeGuestError;

    fn try_from(_value: &[u8]) -> Result<Self, Self::Error> {
        todo!()
    }
}
