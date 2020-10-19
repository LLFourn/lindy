#[derive(Clone)]
pub struct Seed([u8; 32]);

crate::impl_fromstr_deserailize! {
    name => "32-byte lindy seed",
    fn from_bytes(bytes: [u8;32]) -> Option<Seed> {
        Some(Seed(bytes))
    }
}

impl std::fmt::Debug for Seed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", "XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX")
    }
}

// impl Seed {

//     pub fn child(&self, tag: &[u8]) -> Self {
//         Seed(self.to_sha2_hmac().chain(tag).finalize().into())
//     }

//     pub const fn new(bytes: [u8; 32]) -> Self {
//         Self(bytes)
//     }
// }
