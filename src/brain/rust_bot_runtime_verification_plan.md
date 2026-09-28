# Runtime Verification Plan — Running the Real Compiled Bot (DRAFT, not executed)

Status: planning only. Nothing in this plan has been run. Written after a
real-funds test that verified this session's Solend/Kamino on-chain
instruction logic directly (hand-built transactions via `solders`,
`simulateTransaction`-checked, then broadcast and confirmed on mainnet) --
that proved the *on-chain instruction semantics* are correct, but never
exercised the actual compiled Rust/WASM bot (`Wallet`'s transaction
batching/signing, the epoch-driven trigger in `perpfundingv1::state`, the
push-based `on_account` subscription machinery). This plan is about closing
that specific gap: running the real `optimizer perp` command against a real
wallet and watching the actual bot decide, batch, and send transactions
itself.

## Real architecture (verified this session, not assumed)

**Catscope Optimizer coordinates bots via a Solpipe marketplace of
validators — it does not expect the bot operator to run their own
validator.** Confirmed via `$HOME/work/optimizer/CLAUDE.md`: "Catscope
Optimizer coordinates multiple trading bots deployed on Solana validators
via the Solpipe marketplace." The `manager/bidder` subsystem connects to a
**Solpipe bidder daemon** over a Unix socket and exposes `List()` (available
markets/validators) and `Log()` (streamed action logs).

**That bidder daemon is already running in this environment right now** --
confirmed via `ps aux` (`solpipe bidder proxy --fee-payer=./authorizer.json
./proxy.json`, running inside a `tmux` session named `bidder`, cwd
`/home/joel/work/tmp/20260509/bidder`) and live Unix sockets
(`~/.solpipe.bidder.manage.sock`, `~/.solpipe.bidder.proxy.sock`, both
present and socket-typed). This is not leftover/unrelated infrastructure --
it's the real marketplace connection this plan needs, already up.

**`--state-url` is a Solpipe marketplace address, not an arbitrary
endpoint** -- confirmed via `optimizer/cmd/download.go`: it's parsed with
`bidder.ParseAddress(rc.StateURL)`, tying it directly into the same
addressing scheme the bidder daemon uses. The real path to a valid
`--state-url` is almost certainly through the bidder daemon's `List()` (or
an equivalent CLI surface not yet located), not by hand-constructing a URL.

**The Catscope geyser plugin (`/etc/catscope/geyser.json`,
`libsolana_geyser_plugin_catscope.so`) is validator-side infrastructure,
not bot-operator-side** -- it's what a validator loads to *participate* in
the marketplace as a data provider, not something this plan needs to run
directly. No validator process is currently running with it loaded (checked
`ps aux`; the plugin's configured `store_dir`, `/mnt/ledger/catscope`,
doesn't even exist yet), and that's fine -- it's not this plan's job to
start one.

**Running a local validator is currently infeasible anyway, independent of
whether it's the right approach** -- checked real disk space: `/` is at
**99% used, only 5.0GB free**. A real Solana validator ledger needs
hundreds of GB. This forecloses "just run a full local/devnet validator"
as a near-term option regardless of the marketplace question above.

## Real command surface (from `optimizer --help`, already confirmed working)

- `optimizer balance <parent-key> --state-url=<url>` -- read-only, lowest
  risk, good first real-command test once a state-url is in hand.
- `optimizer perp <parent-key> --state-url=<url> --working-dir=<dir>` --
  runs the actual `perpfundingv1` bot (the exact mode tested manually this
  session) against a real wallet.
- `optimizer download-arb <parent-key> --exact=solend,marginfi --force
  --state-url=<url>` -- also closes the *other* real gap found this
  session (Solend's stale main-pool data, marginfi's stale 8-row cache) --
  worth doing once a state-url exists regardless of this plan's main goal.

`<parent-key>` per `--help`: "the file path to the fee payer (not bidder
proxy fee payer)" -- explicitly a *different* key than the bidder proxy's
own `authorizer.json`. For this plan, that should be the same test wallet
(`$HOME/work/optimizer/fee-payer.json`) already used and left in a clean
state this session.

## What's still unknown (real, not glossed over)

- **The exact CLI/API surface for turning the running bidder daemon's
  market list into a real `--state-url` value.** `manager/bidder/manager.go`
  has `List()` at the Go API level; whether `optimizer` exposes this as a
  subcommand, or whether it needs a small Go program written against that
  package, hasn't been checked yet. This is the actual next research step,
  not something to guess at.
- **Whether accessing the marketplace costs real money.** "Bidder" implies
  bidding for validator time/allocation -- if so, running the real bot this
  way has a real cost beyond just Solana tx fees, on top of (or via) the
  `authorizer.json`/`jar`/`sweep` wallets already configured in the running
  proxy's `proxy.json`. Needs to be understood *before* attempting a real
  bid, not discovered by trying it.
- **Whether the already-running bidder proxy is already funded/configured
  for exactly this purpose**, or is unrelated leftover state from other
  work in this environment that happens to be running. Given it's live and
  its sockets are live, it's plausible it's ready to use -- but that's an
  assumption to verify, not a fact yet.

## Phased approach

- **Phase 0** -- Research: read `manager/bidder/manager.go`'s `List()` and
  whatever calls it, to find the real, intended way to obtain a
  `--state-url`. Confirm whether marketplace access has a real cost. Do
  not bid/spend anything during this phase -- read-only investigation only.
- **Phase 1** -- Once a real `--state-url` is in hand: run `optimizer
  balance $HOME/work/optimizer/fee-payer.json --state-url=<url>` (read-only,
  same wallet already used and verified clean this session) and confirm it
  reports the real, correct balance -- first proof the state connection
  itself works before trusting it for anything that sends transactions.
- **Phase 2** -- Run `optimizer download-arb --exact=solend,marginfi
  --force --state-url=<url>` against the real state connection -- closes
  the Solend stale-data gap found this session as a side effect, and is a
  good second-order confidence check on the state connection (a much
  bigger, more varied read than `balance`) before trusting it for
  transaction-sending.
- **Phase 3** -- Run `optimizer perp $HOME/work/optimizer/fee-payer.json
  --state-url=<url> --working-dir=<dir>` against the real, already-tested
  wallet and *observe* -- watch its logs, confirm its bootstrap/idle-USDC
  behavior matches what this session's manual testing already proved
  correct, before ever letting it run unsupervised for an extended period.

## Explicitly not in scope / risks to flag, not hide

- **No bidding or spending on the Solpipe marketplace without a separate,
  explicit go-ahead** -- same standing discipline as every real-funds
  action this session, and more load-bearing here since the cost mechanism
  isn't even understood yet (see "What's still unknown" above).
- **No touching the bidder proxy's own wallets**
  (`authorizer.json`/`jar`/`sweep` in `proxy.json`) -- that's running,
  configured infrastructure this plan doesn't own; it only needs to
  *consume* the marketplace connection, not modify anything about how the
  proxy itself is set up.
- **No letting the real bot run unsupervised once started** -- Phase 3 is
  explicitly framed as "observe," not "deploy and walk away." The bot can
  autonomously send real transactions once running (that's the whole
  point), so the same per-transaction judgment this session applied
  manually needs a real equivalent here (e.g. a tight `working-dir`/log
  review cadence, or a hard stop condition) before any extended unattended
  run -- not designed yet, a real open question for whoever picks this up.
- **Standing local-validator path stays parked, not abandoned** -- if disk
  space is ever freed up meaningfully (currently 5GB free, would need
  hundreds of GB), running a local validator remains a real alternative to
  the marketplace path; not pursued now because it's currently infeasible,
  not because it's wrong.

## Open questions for whoever picks this up

- Is the currently-running `solpipe bidder proxy` actually intended/funded
  for this use case, or should a fresh bidder session be started
  specifically for this test?
- What does marketplace access actually cost, and who's expected to bear
  it?
- Once Phase 3 shows the real bot's behavior matches this session's manual
  verification, is there a plan for longer-running, less-supervised
  operation -- or does every future real-money run get the same
  one-step-at-a-time treatment this whole session used?
