#!/usr/bin/env bash
# Builds a small git repository with a change of every kind, for headless stills and for
# trying the editor by hand:
#
#   scripts/fixture.sh /tmp/code-fixture
#   cargo run -- /tmp/code-fixture
set -euo pipefail
dir="${1:?usage: fixture.sh DIR}"
rm -rf "$dir"
mkdir -p "$dir/src/net" "$dir/web" "$dir/docs" "$dir/target/debug"
cd "$dir"
git init -q -b main
git config user.name "Fixture"
git config user.email "fixture@example.com"
git config core.autocrlf false
git config commit.gpgsign false

# $1: the struct's extra field, $2: its initial value, $3: next()'s parameters,
# $4: the doc comment's tail, $5: the wait, $6: the result, $7: the call's argument.
main_rs() {
  cat <<RS
use std::time::Duration;

mod net;

/// How long to wait before trying a request again.
pub struct Backoff {
    base: Duration,
    max: Duration,
    attempt: u32,$1
}

impl Backoff {
    pub fn new(base: Duration, max: Duration) -> Backoff {
        Backoff { base, max, attempt: 0$2 }
    }

    /// The wait before the next attempt$4.
    pub fn next(&mut self$3) -> Duration {
        let wait = $5;
        self.attempt += 1;
        $6
    }

    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

fn main() {
    let mut backoff = Backoff::new(Duration::from_millis(100), Duration::from_secs(1));
    for attempt in 0..5 {
        println!("attempt {attempt}: wait {:?}", backoff.next($7));
    }
	// A line indented with a tab.
    backoff.reset();
}
RS
}

main_rs "" "" "" "" 'self.base * 2u32.pow(self.attempt)' 'wait.min(self.max)' "" > src/main.rs
echo 'pub mod retry;' > src/net/mod.rs
cat > src/net/retry.rs <<'RS'
pub const TRIES: u32 = 3;

pub fn should_retry(status: u16) -> bool {
    matches!(status, 429 | 500..=599)
}
RS
cat > web/fetch.ts <<'TS'
export async function fetchWithRetry(url: string, tries = 3): Promise<Response> {
  for (let i = 0; i < tries; i++) {
    const res = await fetch(url)
    if (res.ok) return res
  }
  throw new Error(`gave up on ${url}`)
}
TS
printf '// Kept for old browsers.\nmodule.exports = function legacy() { return 1 }\n' > web/legacy.js
printf '# Retry backoff\n\nWaits double from `base_ms` up to `max_ms`.\n' > docs/guide.md
printf '# fixture\n\nA small project for trying the editor.\n' > README.md
printf '{\n  "name": "fixture",\n  "version": "1.0.0"\n}\n' > package.json
printf '/target\n.env\n' > .gitignore
git add -A
git commit -q -m "Initial commit"

# Modified, not staged.
main_rs $'\n    jitter: f32,' ', jitter: 0.2' ', roll: f32' ', give or take `jitter` of it' \
  'self.base.saturating_mul(2u32.saturating_pow(self.attempt))' \
  'wait.min(self.max).mul_f32(1.0 + self.jitter * (roll * 2.0 - 1.0))' '0.5' > src/main.rs
# Modified and staged, with a further change on top that is not.
sed -i 's/tries = 3/tries = 3, signal?: AbortSignal/; s/await fetch(url)/await fetch(url, { signal })/' web/fetch.ts
git add web/fetch.ts
sed -i 's/gave up on \${url}/gave up on ${url} after ${tries} tries/' web/fetch.ts
# Added and staged, renamed and staged, deleted, and untracked (one of them binary).
printf '[retry]\nbase_ms = 100\nmax_ms = 1000\njitter = 0.2\n' > retry.toml
git add retry.toml
git mv docs/guide.md docs/backoff.md
rm web/legacy.js
printf '# Notes\n\n- try jitter\n' > docs/notes.md
printf '\x89PNG\r\n\x1a\n\0\0\0\rIHDR' > docs/curve.png
# Ignored: build output and a secret.
printf 'binary\n' > target/debug/app
printf 'TOKEN=abc\n' > .env
