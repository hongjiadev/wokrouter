pub mod build {
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    pub struct BuildInfo {
        pub product: &'static str,
        pub version: &'static str,
        pub control_protocol: u16,
    }

    impl BuildInfo {
        pub const fn current() -> Self {
            Self {
                product: "WokRouter",
                version: env!("CARGO_PKG_VERSION"),
                control_protocol: 1,
            }
        }
    }
}
