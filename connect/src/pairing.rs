//! Pure trust-transition model, not a TLS implementation or persistent trust store.
//!
//! A future transport must verify possession of a certificate's private key, bind
//! its identity to that certificate and compute/display the v8 verification code.
//! Only then may it call `connected` with the peer certificate's SHA-256 digest.
//! The UI must explicitly approve an incoming request after code verification.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State { Disconnected, Unpaired, Incoming, Outgoing, Accepting, Paired }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Session(u64);
/// Identifies one approval/send transaction, even across retries on the same TLS connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attempt(u64);
#[derive(Debug)]
pub struct Pairing {
    state: State,
    generation: u64,
    peer: Option<[u8; 32]>,
    pin: Option<[u8; 32]>,
    deadline: Option<u64>,
    last_now: u64,
    attempt_counter: u64,
    pending: Option<Attempt>,
}
impl Default for Pairing {
    fn default() -> Self {
        Self { state: State::Disconnected, generation: 0, peer: None, pin: None, deadline: None, last_now: 0, attempt_counter: 0, pending: None }
    }
}
type Result<T> = std::result::Result<T, &'static str>;
impl Attempt { pub fn id(self)->u64 {self.0} }
impl Pairing {
    /// Restore a digest only from the validated owner-private persistent trust store.
    pub fn from_pin(pin:Option<[u8;32]>)->Self {Self{pin,..Self::default()}}
    pub fn pending(&self)->Option<Attempt> {self.pending}
    /// The transport calls this periodically so idle pending prompts expire too.
    pub fn expire(&mut self,session:Session,now:u64) {let _=self.tick(session,now);}

    pub fn state(&self) -> State { self.state }
    pub fn pinned(&self) -> Option<[u8; 32]> { self.pin }
    pub fn authorized(&self) -> bool {
        self.state == State::Paired && self.pin.is_some() && self.peer == self.pin
    }
    pub fn connected(&mut self, tls_certificate_digest: [u8; 32]) -> Result<Session> {
        if self.peer.is_some() { return Err("connection already active"); }
        if self.pin.is_some_and(|pin| pin != tls_certificate_digest) { return Err("certificate changed"); }
        self.generation = self.generation.checked_add(1).ok_or("session counter exhausted")?;
        self.peer = Some(tls_certificate_digest);
        self.deadline = None; self.pending = None; self.last_now = 0;
        self.state = if self.pin.is_some() { State::Paired } else { State::Unpaired };
        Ok(Session(self.generation))
    }
    fn current(&self, session: Session) -> Result<()> {
        if self.peer.is_none() || session.0 != self.generation { Err("stale or disconnected session") } else { Ok(()) }
    }
    fn tick(&mut self, session: Session, now: u64) -> Result<()> {
        self.current(session)?;
        if now < self.last_now { return Err("monotonic clock moved backwards"); }
        self.last_now = now;
        if self.deadline.is_some_and(|end| now >= end) {
            self.state = State::Unpaired; self.deadline = None; self.pending = None;
            return Err("pairing expired");
        }
        Ok(())
    }
    fn begin(&mut self, now: u64, duration: u64, state: State) -> Result<Attempt> {
        let deadline = now.checked_add(duration).ok_or("deadline overflow")?;
        let counter = self.attempt_counter.checked_add(1).ok_or("attempt counter exhausted")?;
        let attempt = Attempt(counter);
        self.attempt_counter = counter; self.pending = Some(attempt);
        self.deadline = Some(deadline); self.state = state;
        Ok(attempt)
    }
    fn attempt(&mut self, session: Session, attempt: Attempt, now: u64) -> Result<()> {
        self.current(session)?;
        if self.pending != Some(attempt) { return Err("stale pairing attempt"); }
        self.tick(session, now)
    }
    /// Monotonic seconds for deadlines; epoch seconds only for v8 clock-skew checks.
    pub fn incoming(&mut self, session: Session, timestamp: i64, wall_now: i64, now: u64) -> Result<Attempt> {
        self.tick(session, now)?;
        if self.state != State::Unpaired { return Err("pairing already active or trusted"); }
        if timestamp < 0 || wall_now < 0 || timestamp.abs_diff(wall_now) > 1800 { return Err("invalid pairing timestamp"); }
        self.begin(now, 25, State::Incoming)
    }
    /// Call only for an explicit local pairing request successfully sent over TLS.
    pub fn request(&mut self, session: Session, now: u64) -> Result<Attempt> {
        self.tick(session, now)?;
        if self.state != State::Unpaired { return Err("pairing already active or trusted"); }
        self.begin(now, 30, State::Outgoing)
    }
    pub fn accept(&mut self, session: Session, attempt: Attempt, now: u64) -> Result<()> {
        self.attempt(session, attempt, now)?;
        if self.state != State::Incoming { return Err("no incoming pairing request"); }
        self.state = State::Accepting; Ok(())
    }
    /// Authorize only after the acceptance packet was successfully sent.
    pub fn accept_sent(&mut self, session: Session, attempt: Attempt, now: u64) -> Result<()> {
        self.attempt(session, attempt, now)?;
        if self.state != State::Accepting { return Err("acceptance is not pending"); }
        self.trust(); Ok(())
    }
    pub fn peer_accepts(&mut self, session: Session, attempt: Attempt, now: u64) -> Result<()> {
        self.attempt(session, attempt, now)?;
        if self.state != State::Outgoing { return Err("no outgoing pairing request"); }
        self.trust(); Ok(())
    }
    fn trust(&mut self) { self.pin = self.peer; self.state = State::Paired; self.deadline = None; self.pending = None; }
    /// Explicit local unpair or an authenticated peer's unpair packet.
    pub fn unpair(&mut self, session: Session) -> Result<()> {
        self.current(session)?;
        self.pin = None; self.deadline = None; self.pending = None; self.state = State::Unpaired; Ok(())
    }
    /// Reject or cancel only the specific pending UI transaction.
    pub fn cancel(&mut self, session: Session, attempt: Attempt) -> Result<()> {
        self.current(session)?;
        if self.pending != Some(attempt) { return Err("stale pairing attempt"); }
        self.pending = None; self.deadline = None; self.state = State::Unpaired; Ok(())
    }
    pub fn send_failed(&mut self, session: Session, attempt: Attempt) {
        let _ = self.cancel(session, attempt);
    }
    pub fn disconnected(&mut self, session: Session) {
        if self.current(session).is_ok() {
            self.peer = None; self.deadline = None; self.pending = None; self.state = State::Disconnected;
        }
    }
}
