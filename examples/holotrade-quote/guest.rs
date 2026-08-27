//! HoloTrade quote engine, ported to the Hologram `core-wasm-v1` guest contract.
//!
//! This is a spike: it takes one self-contained JSON quote request on stdin
//! (via the host's input buffer) and returns the full price decomposition the
//! HoloTrade UI would show for one node — every multiplier, the floor, the
//! margin, and the locality cost computed from the W(3,3) geometry.
//!
//! Ported from https://github.com/wilcompute/Holotrade (MIT):
//!   js/pricing.js    quote(), the six multipliers, the floor
//!   js/energy.js     multiplier(), hourlyEnergyCost(), hourlyCarbon()
//!   js/fleet.js      specialisationScore(), fitness(), maintenanceReserve(),
//!                    capitalRecovery(), CAPEX_MULTIPLE
//!   js/substrate.js  buildPoints(), symplecticForm(), route(),
//!                    fabricDistance(), migrationCost(), magicMultiplier()
//!
//! Build:
//!   rustc --target wasm32-unknown-unknown -O -C panic=abort \
//!     --crate-type cdylib main.rs -o app.wasm
//!
//! The guest uses std but no external crates, so a plain rustc invocation
//! suffices — no Cargo project and no network access required. It imports
//! nothing from the host: the module exports `memory`, `holo_alloc`, and
//! `holo_run` only.

// ---------------------------------------------------------------------
// Guest contract (core-wasm-v1)
// ---------------------------------------------------------------------

#[no_mangle]
pub extern "C" fn holo_alloc(len: i32) -> i32 {
    let mut buf: Vec<u8> = Vec::with_capacity(len.max(0) as usize);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr as i32
}

#[no_mangle]
pub extern "C" fn holo_run(ptr: i32, len: i32) -> i64 {
    let input = unsafe { std::slice::from_raw_parts(ptr as *const u8, len.max(0) as usize) };
    let output = run(input);
    let out_len = output.len() as i64;
    let out_ptr = output.as_ptr() as i64;
    std::mem::forget(output);
    (out_ptr << 32) | out_len
}

fn run(input: &[u8]) -> Vec<u8> {
    match std::str::from_utf8(input)
        .map_err(|e| e.to_string())
        .and_then(parse_json)
        .map(|request| quote(&request))
    {
        Ok(json) => json.into_bytes(),
        Err(message) => {
            let mut out = String::from("{\"error\":");
            push_str(&mut out, &message);
            out.push('}');
            out.into_bytes()
        }
    }
}

// ---------------------------------------------------------------------
// Minimal JSON (parse + emit). The request is machine-generated, so the
// parser keeps to the JSON grammar without chasing edge cosmetics.
// ---------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    fn f64(&self) -> Option<f64> {
        match self {
            Json::Num(x) => Some(*x),
            _ => None,
        }
    }
    fn bool_or(&self, default: bool) -> bool {
        match self {
            Json::Bool(b) => *b,
            _ => default,
        }
    }
    fn string(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    fn array(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(items) => Some(items),
            _ => None,
        }
    }
}

fn parse_json(text: &str) -> Result<Json, String> {
    let mut p = JsonParser {
        bytes: text.as_bytes(),
        pos: 0,
    };
    p.skip_ws();
    let value = p.value()?;
    p.skip_ws();
    if p.pos != p.bytes.len() {
        return Err(format!("trailing bytes at offset {}", p.pos));
    }
    Ok(value)
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }
    fn expect(&mut self, byte: u8) -> Result<(), String> {
        if self.peek() == Some(byte) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!(
                "expected '{}' at offset {}",
                byte as char, self.pos
            ))
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.literal("true", Json::Bool(true)),
            Some(b'f') => self.literal("false", Json::Bool(false)),
            Some(b'n') => self.literal("null", Json::Null),
            Some(b'-') | Some(b'0'..=b'9') => self.number(),
            other => Err(format!(
                "unexpected byte {:?} at offset {}",
                other, self.pos
            )),
        }
    }
    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(format!("invalid literal at offset {}", self.pos))
        }
    }
    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while self.pos < self.bytes.len()
            && matches!(
                self.bytes[self.pos],
                b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9'
            )
        {
            self.pos += 1;
        }
        std::str::from_utf8(&self.bytes[start..self.pos])
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .map(Json::Num)
            .ok_or_else(|| format!("invalid number at offset {}", start))
    }
    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("unterminated string".to_string()),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let esc = self.peek().ok_or("unterminated escape")?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'u' => {
                            let hex = self
                                .bytes
                                .get(self.pos..self.pos + 4)
                                .and_then(|h| std::str::from_utf8(h).ok())
                                .and_then(|h| u32::from_str_radix(h, 16).ok())
                                .ok_or("invalid \\u escape")?;
                            self.pos += 4;
                            out.push(char::from_u32(hex).unwrap_or('\u{FFFD}'));
                        }
                        other => return Err(format!("invalid escape '{}'", other as char)),
                    }
                }
                Some(_) => {
                    // Consume one UTF-8 code point verbatim.
                    let rest = &self.bytes[self.pos..];
                    let ch_len = std::str::from_utf8(rest)
                        .ok()
                        .and_then(|s| s.chars().next())
                        .map(|c| c.len_utf8())
                        .unwrap_or(1);
                    let end = (self.pos + ch_len).min(self.bytes.len());
                    out.push_str(&String::from_utf8_lossy(&self.bytes[self.pos..end]));
                    self.pos = end;
                }
            }
        }
    }
    fn object(&mut self) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Obj(entries));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(b':')?;
            let value = self.value()?;
            entries.push((key, value));
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(entries));
                }
                other => {
                    return Err(format!(
                        "expected ',' or '}}' at offset {}, got {:?}",
                        self.pos, other
                    ))
                }
            }
        }
    }
    fn array(&mut self) -> Result<Json, String> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                other => {
                    return Err(format!(
                        "expected ',' or ']' at offset {}, got {:?}",
                        self.pos, other
                    ))
                }
            }
        }
    }
}

fn fmt_num(x: f64) -> String {
    if x.is_finite() {
        format!("{}", x)
    } else {
        "null".to_string()
    }
}

fn push_num(out: &mut String, key: &str, value: f64) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    out.push_str(&fmt_num(value));
}

fn push_opt_num(out: &mut String, key: &str, value: Option<f64>) {
    out.push('"');
    out.push_str(key);
    out.push_str("\":");
    match value {
        Some(x) if x.is_finite() => out.push_str(&fmt_num(x)),
        _ => out.push_str("null"),
    }
}

fn push_str(out: &mut String, value: &str) {
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------
// W(3,3) substrate (js/substrate.js)
// ---------------------------------------------------------------------

const RAY_IN_PLACE: f64 = 6.0;
const RAY_ADJACENT: f64 = 3.0;
const RAY_NON_ADJACENT: f64 = 5.0;
const HOPS_PER_DIGIT: f64 = 8.0;

fn build_points() -> Vec<[i32; 4]> {
    let mut seen: Vec<[i32; 4]> = Vec::new();
    let mut points: Vec<[i32; 4]> = Vec::new();
    for a in 0i32..3 {
        for b in 0i32..3 {
            for c in 0i32..3 {
                for d in 0i32..3 {
                    let v = [a, b, c, d];
                    if v.iter().all(|&x| x == 0) {
                        continue;
                    }
                    let lead = *v.iter().find(|&&x| x != 0).unwrap();
                    let inv = if lead == 1 { 1 } else { 2 }; // 2*2 = 1 mod 3
                    let norm = [
                        (v[0] * inv).rem_euclid(3),
                        (v[1] * inv).rem_euclid(3),
                        (v[2] * inv).rem_euclid(3),
                        (v[3] * inv).rem_euclid(3),
                    ];
                    if seen.contains(&norm) {
                        continue;
                    }
                    seen.push(norm);
                    points.push(norm);
                }
            }
        }
    }
    points
}

fn symplectic_form(u: &[i32; 4], v: &[i32; 4]) -> i32 {
    (u[0] * v[1] - u[1] * v[0] + u[2] * v[3] - u[3] * v[2]).rem_euclid(3)
}

fn is_adjacent(points: &[[i32; 4]], a: i64, b: i64) -> bool {
    if a < 0 || b < 0 || a >= points.len() as i64 || b >= points.len() as i64 || a == b {
        return false;
    }
    symplectic_form(&points[a as usize], &points[b as usize]) == 0
}

/// In-cell route distance: 0 same point, 1 direct edge, 2 via one of the
/// mu = 4 common neighbours (guaranteed to exist on SRG(40,12,2,4)).
fn route_distance(points: &[[i32; 4]], a: i64, b: i64) -> f64 {
    if a == b {
        0.0
    } else if is_adjacent(points, a, b) {
        1.0
    } else {
        2.0
    }
}

fn shared_prefix(a: &[i64], b: &[i64]) -> usize {
    let mut i = 0;
    while i < a.len() && i < b.len() && a[i] == b[i] {
        i += 1;
    }
    i
}

fn fabric_distance(points: &[[i32; 4]], a: &[i64], b: &[i64]) -> (f64, usize) {
    let i = shared_prefix(a, b);
    if i == a.len() && i == b.len() {
        return (0.0, i);
    }
    let in_cell = match (a.get(i), b.get(i)) {
        (Some(&da), Some(&db)) => route_distance(points, da, db),
        _ => 1.0,
    };
    let depth_below =
        (a.len() as f64 - i as f64 - 1.0).max(0.0) + (b.len() as f64 - i as f64 - 1.0).max(0.0);
    (in_cell + depth_below * HOPS_PER_DIGIT, i)
}

fn migration_rays(points: &[[i32; 4]], from: i64, to: i64) -> f64 {
    if from == to {
        RAY_IN_PLACE
    } else if is_adjacent(points, from, to) {
        RAY_ADJACENT
    } else {
        RAY_NON_ADJACENT
    }
}

/// (rays, hops, channel)
fn migration_cost(points: &[[i32; 4]], a: &[i64], b: &[i64]) -> (f64, f64, &'static str) {
    let (hops, prefix) = fabric_distance(points, a, b);
    let i = prefix;
    let base = if i >= a.len() || i >= b.len() {
        RAY_IN_PLACE
    } else {
        migration_rays(points, a[i], b[i])
    };
    let depth_penalty = if prefix == 0 {
        (a.len() as f64 - 1.0).max(0.0) * 2.0
    } else {
        0.0
    };
    let rays = base + depth_penalty;
    let channel = if i >= a.len() || i >= b.len() {
        "in-place"
    } else if rays == RAY_ADJACENT && hops <= 1.0 {
        "cheap"
    } else if hops <= 2.0 {
        "in-cell"
    } else {
        "far"
    };
    (rays, hops, channel)
}

fn magic_multiplier(t: f64) -> f64 {
    9f64.powf(t.max(0.0))
}

// ---------------------------------------------------------------------
// Fleet model (js/fleet.js)
// ---------------------------------------------------------------------

fn capex_multiple(kind: &str) -> f64 {
    match kind {
        "gpu" => 1100.0,
        "cpu" => 950.0,
        "fpga" => 1400.0,
        "neuro" => 1600.0,
        "photonic" => 2600.0,
        "composite" => 1100.0,
        _ => 900.0,
    }
}

fn clamp01(x: f64) -> f64 {
    x.max(0.0).min(1.0)
}

fn clamp_to(x: f64, lo: f64, hi: f64) -> f64 {
    if !x.is_finite() {
        return x;
    }
    x.max(lo).min(hi)
}

fn specialisation_score(node: &Json, workload: &Json) -> f64 {
    let workload_id = workload.get("id").and_then(Json::string).unwrap_or("");
    let prior = node
        .get("specialisation")
        .and_then(|s| s.get(workload_id))
        .and_then(Json::f64)
        .unwrap_or(0.2);
    let gene_avg = match workload.get("geneEmphasis").and_then(Json::array) {
        Some(genes) if !genes.is_empty() => {
            let sum: f64 = genes
                .iter()
                .filter_map(Json::string)
                .map(|g| {
                    node.get("genome")
                        .and_then(|genome| genome.get(g))
                        .and_then(Json::f64)
                        .unwrap_or(0.5)
                })
                .sum();
            sum / genes.len() as f64
        }
        _ => 0.5,
    };
    let magic_budget = workload
        .get("magicBudget")
        .and_then(Json::f64)
        .unwrap_or(0.0);
    let magic_capable = node
        .get("hardware")
        .and_then(|h| h.get("magicCapable"))
        .map(|j| j.bool_or(false))
        .unwrap_or(false);
    if magic_budget > 0.0 && !magic_capable {
        return 0.0;
    }
    clamp01(0.55 * prior + 0.45 * gene_avg)
}

fn fitness(node: &Json, workloads: &[Json]) -> f64 {
    let completed = node.get("jobsCompleted").and_then(Json::f64).unwrap_or(0.0);
    let failed = node.get("jobsFailed").and_then(Json::f64).unwrap_or(0.0);
    let attempts = completed + failed;
    let completion = if attempts > 0.0 {
        completed / attempts
    } else {
        0.5
    };
    // bestClass: strictly-greater scan, initial score -1.
    let mut best = -1.0;
    for w in workloads {
        let s = specialisation_score(node, w);
        if s > best {
            best = s;
        }
    }
    let genome = node.get("genome");
    let gene = |name: &str| {
        genome
            .and_then(|g| g.get(name))
            .and_then(Json::f64)
            .unwrap_or(0.5)
    };
    let gene_quality =
        (gene("throughput") + gene("convergenceRate") + gene("faultResilience")) / 3.0;
    let experience = ((1.0 + completed).log10() / 3.2).min(1.0);
    let raw = 0.34 * completion + 0.26 * best + 0.22 * gene_quality + 0.18 * experience;
    let derate = node
        .get("health")
        .and_then(|h| h.get("derate"))
        .and_then(Json::f64)
        .unwrap_or(1.0);
    clamp01(raw * derate)
}

// ---------------------------------------------------------------------
// The quote (js/pricing.js + js/energy.js)
// ---------------------------------------------------------------------

const TARGET_LOW: f64 = 0.55;
const TARGET_HIGH: f64 = 0.78;
const BAND_TOP: f64 = 1.10;

struct QuoteInput<'a> {
    node: &'a Json,
    workloads: &'a [Json],
    workload: Option<&'a Json>,         // genetics fallback: workloads[0]
    quantum_workload: Option<&'a Json>, // quantum: find only, no fallback
    anchor: Option<Vec<i64>>,
    node_addr: Vec<i64>,
    energy_price: f64,
    energy_base: f64,
    carbon_intensity: f64,
    pue: f64,
    balancer_enabled: bool,
}

fn field(node: &Json, path: &[&str], default: f64) -> f64 {
    let mut cur = node;
    for key in path {
        match cur.get(key) {
            Some(next) => cur = next,
            None => return default,
        }
    }
    cur.f64().unwrap_or(default)
}

fn quote(request: &Json) -> String {
    let node = request
        .get("node")
        .cloned()
        .unwrap_or(Json::Obj(Vec::new()));
    let empty: Vec<Json> = Vec::new();
    let workloads = request
        .get("workloads")
        .and_then(Json::array)
        .unwrap_or(&empty);
    let workload_id = request
        .get("workloadId")
        .and_then(Json::string)
        .unwrap_or("llm-train");
    let find_workload = || {
        workloads
            .iter()
            .find(|w| w.get("id").and_then(Json::string) == Some(workload_id))
    };

    let anchor: Option<Vec<i64>> = match request.get("anchorAddress") {
        None | Some(Json::Null) => None,
        Some(Json::Arr(items)) => Some(
            items
                .iter()
                .filter_map(Json::f64)
                .map(|x| x as i64)
                .collect(),
        ),
        _ => None,
    };
    let node_addr: Vec<i64> = match node.get("addr") {
        Some(Json::Arr(items)) => items
            .iter()
            .filter_map(Json::f64)
            .map(|x| x as i64)
            .collect(),
        _ => Vec::new(),
    };
    let energy = request
        .get("energy")
        .cloned()
        .unwrap_or(Json::Obj(Vec::new()));

    let input = QuoteInput {
        node: &node,
        workloads,
        workload: find_workload().or_else(|| workloads.first()),
        quantum_workload: find_workload(),
        anchor,
        node_addr,
        energy_price: energy.get("price").and_then(Json::f64).unwrap_or(0.0),
        energy_base: energy.get("baseEnergy").and_then(Json::f64).unwrap_or(1.0),
        carbon_intensity: energy.get("carbon").and_then(Json::f64).unwrap_or(0.0),
        pue: energy.get("pue").and_then(Json::f64).unwrap_or(1.0),
        balancer_enabled: request
            .get("balancerEnabled")
            .map(|j| j.bool_or(true))
            .unwrap_or(true),
    };

    let points = build_points();

    let base = field(node_ref(&input), &["hardware", "baseRate"], 0.0);
    let utilisation = field(node_ref(&input), &["utilisation"], 0.0);
    let utilisation_ema = field(node_ref(&input), &["utilisationEMA"], utilisation);
    let thermal_sensitivity = field(node_ref(&input), &["hardware", "thermalSensitivity"], 1.0);
    let tdp = field(node_ref(&input), &["hardware", "tdp"], 0.0);
    let life_hours = field(node_ref(&input), &["hardware", "lifeHours"], 1.0);
    let magic_capable = node_ref(&input)
        .get("hardware")
        .and_then(|h| h.get("magicCapable"))
        .map(|j| j.bool_or(false))
        .unwrap_or(false);
    let derate = field(node_ref(&input), &["health", "derate"], 1.0);
    let hazard = field(node_ref(&input), &["health", "hazard"], 0.0);
    let correctable_errors = field(node_ref(&input), &["health", "correctableErrors"], 0.0);
    let wear = field(node_ref(&input), &["health", "wear"], 0.0);
    let generation = field(node_ref(&input), &["lineage", "generation"], 0.0);
    let jobs_completed = field(node_ref(&input), &["jobsCompleted"], 0.0);
    let kind = node_ref(&input)
        .get("hardware")
        .and_then(|h| h.get("kind"))
        .and_then(Json::string)
        .unwrap_or("")
        .to_string();

    // E -- energy (energy.js multiplier, then pricing.js clamp; same range)
    let ratio = (input.energy_price / input.energy_base).max(0.05);
    let e = clamp_to(ratio.powf(0.55), 0.62, 2.4);

    // G -- genetics
    let g = match input.workload {
        Some(w) => {
            let spec = specialisation_score(node_ref(&input), w);
            if spec == 0.0 {
                0.0
            } else {
                let fit = fitness(node_ref(&input), input.workloads);
                let provenance = (generation / 6.0).min(1.0) * 0.5
                    + ((1.0 + jobs_completed).log10() / 3.2).min(1.0) * 0.5;
                clamp_to(
                    0.55 + 1.05 * spec + 0.55 * fit + 0.25 * provenance,
                    0.7,
                    2.6,
                )
            }
        }
        None => 0.0,
    };

    // D -- demand / wear (the two-sided balancer)
    let d = if !input.balancer_enabled {
        1.0
    } else if utilisation > TARGET_HIGH {
        let over = (utilisation - TARGET_HIGH) / (1.0 - TARGET_HIGH);
        let scarcity = 1.0 + 1.35 * over.powf(1.4);
        let wear_rate = 0.45 + 1.1 * utilisation + (utilisation - utilisation_ema).abs() * 3.2;
        let surcharge = 1.0 + 0.28 * (wear_rate - 1.2) * thermal_sensitivity;
        clamp_to(BAND_TOP * scarcity * surcharge.max(1.0), 0.55, 3.2)
    } else if utilisation < TARGET_LOW {
        let under = (TARGET_LOW - utilisation) / TARGET_LOW;
        clamp_to(1.0 - 0.42 * under.powf(0.85), 0.55, 3.2)
    } else {
        let t = (utilisation - TARGET_LOW) / (TARGET_HIGH - TARGET_LOW);
        1.0 + t * (BAND_TOP - 1.0)
    };

    // H -- health
    let reliability = 1.0 - (hazard * 1.6).min(0.3);
    let error_drift = 1.0 - (correctable_errors / 40000.0).min(0.08);
    let h = clamp_to(derate * reliability * error_drift, 0.58, 1.12);

    // Q -- quantum (find only, no fallback; Infinity when unservable)
    let q = match input.quantum_workload {
        Some(w) => {
            let t = w.get("magicBudget").and_then(Json::f64).unwrap_or(0.0);
            if t <= 0.0 {
                1.0
            } else if !magic_capable {
                f64::INFINITY
            } else {
                1.0 + 0.34 * magic_multiplier(t).ln()
            }
        }
        None => 1.0,
    };

    // L -- locality (fabric distance against the anchor, if any)
    let (l, locality_detail) = match &input.anchor {
        None => (1.0, None),
        Some(anchor) => {
            let (rays, hops, channel) = migration_cost(&points, anchor, &input.node_addr);
            let hop_term = 1.0 + hops * 0.018;
            let ray_term = 1.0 + (rays - RAY_ADJACENT) * 0.035;
            (
                clamp_to(hop_term * ray_term, 0.92, 1.85),
                Some((rays, hops, channel)),
            )
        }
    };

    let serviceable = g > 0.0 && q.is_finite();
    let raw_price = if serviceable {
        Some(base * e * g * d * h * q * l)
    } else {
        None
    };

    // Floor: energy + maintenance reserve + capital recovery
    let kw = (tdp / 1000.0) * input.pue * (0.32 + 0.68 * utilisation);
    let energy_cost = kw * input.energy_price * 0.001;
    let service_cost = base * 6.0 + 180.0;
    let expected_hours = ((0.72 - wear) * life_hours * 0.4).max(60.0);
    let reserve = (service_cost / expected_hours) * (1.0 + hazard * 8.0);
    let capex = base * capex_multiple(&kind);
    let capital = capex / (life_hours * 0.72 * derate).max(1000.0);
    let floor = energy_cost + reserve + capital;

    let price = raw_price.map(|p| p.max(floor * 1.02));
    let carbon_per_hour = kw * input.carbon_intensity / 1000.0;

    // ---- emit (field order mirrors js/pricing.js quote()) ----
    let mut out = String::from("{");
    out.push_str("\"nodeId\":");
    push_str(
        &mut out,
        node_ref(&input)
            .get("id")
            .and_then(Json::string)
            .unwrap_or(""),
    );
    out.push_str(",\"workloadId\":");
    push_str(&mut out, workload_id);
    out.push_str(",\"serviceable\":");
    out.push_str(if serviceable { "true" } else { "false" });
    out.push(',');
    push_num(&mut out, "base", base);
    out.push_str(",\"multipliers\":{");
    push_num(&mut out, "E", e);
    out.push(',');
    push_num(&mut out, "G", g);
    out.push(',');
    push_num(&mut out, "D", d);
    out.push(',');
    push_num(&mut out, "H", h);
    out.push(',');
    push_opt_num(&mut out, "Q", if q.is_finite() { Some(q) } else { None });
    out.push(',');
    push_num(&mut out, "L", l);
    out.push('}');
    out.push(',');
    push_opt_num(&mut out, "price", price);
    out.push(',');
    push_opt_num(&mut out, "rawPrice", raw_price);
    out.push(',');
    push_num(&mut out, "floor", floor);
    out.push(',');
    push_num(&mut out, "energyCost", energy_cost);
    out.push(',');
    push_num(&mut out, "maintenanceReserve", reserve);
    out.push(',');
    push_num(&mut out, "capitalRecovery", capital);
    out.push(',');
    push_opt_num(&mut out, "margin", price.map(|p| p - floor));
    out.push(',');
    push_opt_num(&mut out, "marginPct", price.map(|p| (p - floor) / p));
    out.push(',');
    push_num(&mut out, "carbonPerHour", carbon_per_hour);
    out.push_str(",\"atFloor\":");
    out.push_str(match (serviceable, raw_price) {
        (true, Some(p)) if p < floor * 1.02 => "true",
        _ => "false",
    });
    out.push_str(",\"locality\":");
    match locality_detail {
        Some((rays, hops, channel)) => {
            out.push('{');
            push_num(&mut out, "hops", hops);
            out.push(',');
            push_num(&mut out, "rays", rays);
            out.push_str(",\"channel\":");
            push_str(&mut out, channel);
            out.push('}');
        }
        None => out.push_str("null"),
    }
    out.push('}');
    out
}

fn node_ref<'a>(input: &'a QuoteInput) -> &'a Json {
    input.node
}
