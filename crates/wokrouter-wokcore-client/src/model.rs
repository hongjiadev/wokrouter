use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CoreConnection {
    Missing,
    Stopped,
    Running(CoreHandshake),
    Incompatible(Compatibility),
    InvalidRuntime,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Compatibility {
    pub wokcore_minimum_api_major: u32,
    pub wokcore_maximum_api_major: u32,
    pub wokrouter_minimum_api_major: u32,
    pub wokrouter_maximum_api_major: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreHandshake {
    pub instance_id: String,
    pub version: String,
    pub management_api_major: u32,
    pub provider_protocols: BTreeSet<String>,
    pub capabilities: BTreeSet<String>,
}

impl Compatibility {
    pub(crate) fn for_discovered_major(wokcore: u32, wokrouter: u32) -> Self {
        Self {
            wokcore_minimum_api_major: wokcore,
            wokcore_maximum_api_major: wokcore,
            wokrouter_minimum_api_major: wokrouter,
            wokrouter_maximum_api_major: wokrouter,
        }
    }

    pub(crate) fn overlaps(&self) -> bool {
        self.wokcore_minimum_api_major <= self.wokrouter_maximum_api_major
            && self.wokrouter_minimum_api_major <= self.wokcore_maximum_api_major
    }
}
