#[derive(Debug, Default)]
#[repr(C, align(8))]
pub(crate) struct Configuration {
    pub(crate) count: usize,
}
