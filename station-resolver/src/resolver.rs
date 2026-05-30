use radio_core::Callsign;

use crate::resolution::Resolution;

pub trait Resolver: std::fmt::Debug + Send + Sync {
    fn resolve(&self, call: &Callsign) -> Option<Resolution>;
}

#[derive(Debug)]
pub struct NoOpResolver;

impl Resolver for NoOpResolver {
    fn resolve(&self, _call: &Callsign) -> Option<Resolution> {
        None
    }
}
