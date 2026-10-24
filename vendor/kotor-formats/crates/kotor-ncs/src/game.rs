/// Which game's action table a script was compiled against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Game {
    /// Knights of the Old Republic (action ids 0..=771).
    K1,
    /// The Sith Lords (full action table).
    K2,
}
