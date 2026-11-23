//! Variable scopes and `{{variable}}` substitution — mirrors postman-collection's
//! Substitutor (precedence: local > data > environment > collection > globals,
//! up to 19 nested substitution passes, `{{$dynamic}}` generators).

use rand::seq::SliceRandom;
use rand::Rng;
use serde_json::{Map, Value};

use crate::model::{value_to_string, EnvVar, Environment, Variable};

#[derive(Clone, Debug, Default)]
pub struct Scope {
    pub vars: Vec<(String, Value, bool)>, // key, value, enabled
}

impl Scope {
    pub fn from_env(e: &Environment) -> Self {
        Scope { vars: e.values.iter().map(|v| (v.key.clone(), v.value.clone(), v.enabled)).collect() }
    }
    pub fn from_vars(v: &[Variable]) -> Self {
        Scope { vars: v.iter().map(|v| (v.key.clone(), v.value.clone(), !v.disabled)).collect() }
    }
    pub fn from_map(m: &Map<String, Value>) -> Self {
        Scope { vars: m.iter().map(|(k, v)| (k.clone(), v.clone(), true)).collect() }
    }
    pub fn get(&self, k: &str) -> Option<&Value> {
        // last definition wins (like VariableList)
        self.vars.iter().rev().find(|(key, _, en)| *en && key == k).map(|(_, v, _)| v)
    }
    pub fn set(&mut self, k: &str, v: Value) {
        if let Some(e) = self.vars.iter_mut().rev().find(|(key, _, _)| key == k) {
            e.1 = v;
            e.2 = true;
        } else {
            self.vars.push((k.to_string(), v, true));
        }
    }
    pub fn unset(&mut self, k: &str) {
        self.vars.retain(|(key, _, _)| key != k);
    }
    pub fn to_json(&self) -> Value {
        let mut m = Map::new();
        for (k, v, en) in &self.vars {
            if *en {
                m.insert(k.clone(), v.clone());
            }
        }
        Value::Object(m)
    }
    /// Replace content from JS object (keeps disabled entries, order of existing keys)
    pub fn update_from_json(&mut self, v: &Value) {
        if let Value::Object(m) = v {
            // remove enabled keys that are gone
            self.vars.retain(|(k, _, en)| !*en || m.contains_key(k));
            for (k, val) in m {
                self.set(k, val.clone());
            }
        }
    }
    /// Write values back into an environment (preserving types/secret flags)
    pub fn write_env(&self, e: &mut Environment) {
        let mut out: Vec<EnvVar> = Vec::new();
        for (k, v, en) in &self.vars {
            let mut ev = e.values.iter().find(|x| &x.key == k).cloned().unwrap_or_else(|| EnvVar::new(k, ""));
            ev.value = v.clone();
            ev.enabled = *en;
            out.push(ev);
        }
        e.values = out;
    }
    pub fn write_vars(&self, vars: &mut Vec<Variable>) {
        let mut out = Vec::new();
        for (k, v, en) in &self.vars {
            let mut nv = vars.iter().find(|x| &x.key == k).cloned().unwrap_or_else(|| Variable { key: k.clone(), ..Default::default() });
            nv.value = v.clone();
            nv.disabled = !*en;
            out.push(nv);
        }
        *vars = out;
    }
}

/// All scopes in precedence order.
#[derive(Clone, Debug, Default)]
pub struct VarContext {
    pub locals: Scope,
    pub data: Scope,
    pub environment: Scope,
    pub env_name: String,
    pub collection: Scope,
    pub globals: Scope,
    /// path variables of current request (`:id`)
    pub path_vars: Scope,
}

impl VarContext {
    pub fn lookup(&self, key: &str) -> Option<String> {
        for s in [&self.locals, &self.data, &self.environment, &self.collection, &self.globals] {
            if let Some(v) = s.get(key) {
                return Some(value_to_string(v));
            }
        }
        if key.starts_with('$') {
            return dynamic_variable(key);
        }
        None
    }

    /// Which scope defines the key (for UI hover)
    pub fn source_of(&self, key: &str) -> Option<&'static str> {
        let names = ["Local", "Data", "Environment", "Collection", "Global"];
        for (i, s) in [&self.locals, &self.data, &self.environment, &self.collection, &self.globals].iter().enumerate() {
            if s.get(key).is_some() {
                return Some(names[i]);
            }
        }
        if key.starts_with('$') && dynamic_variable(key).is_some() {
            return Some("Dynamic");
        }
        None
    }

    pub fn resolve(&self, s: &str) -> String {
        resolve_with(s, |k| self.lookup(k))
    }
}

/// Replace `{{name}}` occurrences, repeating up to 19 passes for nested vars.
pub fn resolve_with<F: Fn(&str) -> Option<String>>(input: &str, lookup: F) -> String {
    if !input.contains("{{") {
        return input.to_string();
    }
    let mut cur = input.to_string();
    for _ in 0..19 {
        let mut out = String::with_capacity(cur.len());
        let mut replaced = false;
        let mut rest = cur.as_str();
        while let Some(start) = rest.find("{{") {
            let after = &rest[start + 2..];
            // token may not contain { or }
            match after.find("}}") {
                Some(end) if !after[..end].contains('{') && !after[..end].contains('}') => {
                    let name = &after[..end];
                    out.push_str(&rest[..start]);
                    match lookup(name) {
                        Some(v) => {
                            out.push_str(&v);
                            replaced = true;
                        }
                        None => {
                            out.push_str("{{");
                            out.push_str(name);
                            out.push_str("}}");
                        }
                    }
                    rest = &after[end + 2..];
                }
                _ => {
                    out.push_str(&rest[..start + 2]);
                    rest = after;
                }
            }
        }
        out.push_str(rest);
        cur = out;
        if !replaced || !cur.contains("{{") {
            break;
        }
    }
    cur
}

/// Extract variable names used in a string
pub fn used_vars(s: &str) -> Vec<(usize, usize, String)> {
    let mut out = vec![];
    let mut i = 0;
    while let Some(st) = s[i..].find("{{") {
        let start = i + st;
        let after = &s[start + 2..];
        match after.find("}}") {
            Some(end) if !after[..end].contains('{') => {
                out.push((start, start + 2 + end + 2, after[..end].to_string()));
                i = start + 2 + end + 2;
            }
            _ => break,
        }
    }
    out
}

// ---------------------------------------------------------------- dynamic variables

const FIRST: &[&str] = &["James", "Mary", "John", "Patricia", "Robert", "Jennifer", "Michael", "Linda", "William", "Elizabeth", "David", "Barbara", "Richard", "Susan", "Joseph", "Jessica", "Thomas", "Sarah", "Charles", "Karen", "Anna", "Peter", "Eva", "Adam", "Zoe"];
const LAST: &[&str] = &["Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Rodriguez", "Martinez", "Hernandez", "Lopez", "Wilson", "Anderson", "Thomas", "Taylor", "Moore", "Jackson", "Martin", "Lee", "Kovacs", "Nagy"];
const WORDS: &[&str] = &["lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do", "eiusmod", "tempor", "incididunt", "ut", "labore", "et", "dolore", "magna", "aliqua", "enim", "minim", "veniam", "quis", "nostrud", "exercitation", "ullamco", "laboris", "nisi", "aliquip", "commodo"];
const NOUNS: &[&str] = &["system", "matrix", "protocol", "bandwidth", "array", "interface", "monitor", "panel", "driver", "circuit", "feed", "program", "alarm", "capacitor", "card", "pixel", "sensor", "firewall", "port", "bus", "microchip", "transmitter", "application", "hard drive"];
const ADJ: &[&str] = &["auxiliary", "primary", "back-end", "digital", "open-source", "virtual", "cross-platform", "redundant", "online", "haptic", "multi-byte", "bluetooth", "wireless", "1080p", "neural", "optical", "solid state", "mobile"];
const VERBS: &[&str] = &["back up", "bypass", "hack", "override", "compress", "copy", "navigate", "index", "connect", "generate", "quantify", "calculate", "synthesize", "input", "transmit", "program", "reboot", "parse"];
const INGVERBS: &[&str] = &["backing up", "bypassing", "hacking", "overriding", "compressing", "copying", "navigating", "indexing", "connecting", "generating", "quantifying", "calculating", "synthesizing", "transmitting", "programming", "parsing"];
const CITIES: &[&str] = &["Budapest", "Vienna", "Berlin", "Paris", "London", "New York", "Tokyo", "Madrid", "Rome", "Prague", "Warsaw", "Amsterdam", "Lisbon", "Oslo", "Chicago", "Toronto"];
const COUNTRIES: &[(&str, &str)] = &[("Hungary", "HU"), ("Austria", "AT"), ("Germany", "DE"), ("France", "FR"), ("United Kingdom", "GB"), ("United States", "US"), ("Japan", "JP"), ("Spain", "ES"), ("Italy", "IT"), ("Canada", "CA"), ("Poland", "PL"), ("Netherlands", "NL")];
const STREETS: &[&str] = &["Main Street", "Oak Avenue", "Maple Drive", "Cedar Lane", "Pine Road", "Elm Street", "Washington Blvd", "Lake View", "Hill Road", "Park Avenue"];
const COLORS: &[&str] = &["red", "green", "blue", "yellow", "purple", "orange", "black", "white", "cyan", "magenta", "silver", "gold", "teal", "lime", "maroon", "navy"];
const DEPTS: &[&str] = &["Books", "Movies", "Music", "Games", "Electronics", "Computers", "Home", "Garden", "Tools", "Grocery", "Health", "Beauty", "Toys", "Kids", "Sports", "Outdoors", "Clothing", "Shoes", "Jewelery", "Automotive", "Industrial"];
const PRODUCTS: &[&str] = &["Chair", "Car", "Computer", "Keyboard", "Mouse", "Bike", "Ball", "Gloves", "Pants", "Shirt", "Table", "Shoes", "Hat", "Towels", "Soap", "Tuna", "Chicken", "Fish", "Cheese", "Bacon", "Pizza", "Salad", "Sausages", "Chips"];
const MATERIALS: &[&str] = &["Steel", "Wooden", "Concrete", "Plastic", "Cotton", "Granite", "Rubber", "Metal", "Soft", "Fresh", "Frozen"];
const PADJ: &[&str] = &["Small", "Ergonomic", "Rustic", "Intelligent", "Gorgeous", "Incredible", "Fantastic", "Practical", "Sleek", "Awesome", "Generic", "Handcrafted", "Handmade", "Licensed", "Refined", "Unbranded", "Tasty"];
const COMPANY_SUFFIX: &[&str] = &["Inc", "and Sons", "LLC", "Group", "Kft.", "Zrt.", "GmbH", "Ltd"];
const CURRENCIES: &[(&str, &str, &str)] = &[("USD", "US Dollar", "$"), ("EUR", "Euro", "€"), ("HUF", "Forint", "Ft"), ("GBP", "Pound Sterling", "£"), ("JPY", "Yen", "¥"), ("CHF", "Swiss Franc", "CHF")];
const MONTHS: &[&str] = &["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
const WEEKDAYS: &[&str] = &["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const DOMAIN_SUFFIX: &[&str] = &["com", "net", "org", "info", "biz", "io", "hu", "name"];
const JOB_DESC: &[&str] = &["Lead", "Senior", "Direct", "Corporate", "Dynamic", "Future", "Product", "National", "Regional", "District", "Central", "Global", "Customer", "Investor", "Dynamic", "International", "Legacy", "Forward", "Internal", "Human", "Chief", "Principal"];
const JOB_AREA: &[&str] = &["Solutions", "Program", "Brand", "Security", "Research", "Marketing", "Directives", "Implementation", "Integration", "Functionality", "Response", "Paradigm", "Tactics", "Identity", "Markets", "Group", "Division", "Applications", "Optimization", "Operations", "Infrastructure", "Intranet", "Communications", "Web", "Quality", "Assurance", "Mobility", "Accounts", "Data", "Creative", "Configuration", "Accountability", "Interactions", "Factors", "Usability", "Metrics"];
const JOB_TYPE: &[&str] = &["Supervisor", "Associate", "Executive", "Liaison", "Officer", "Manager", "Engineer", "Specialist", "Director", "Coordinator", "Administrator", "Architect", "Analyst", "Designer", "Planner", "Orchestrator", "Technician", "Developer", "Producer", "Consultant", "Assistant", "Facilitator", "Agent", "Representative", "Strategist"];
const MIME: &[(&str, &str)] = &[("application/json", "json"), ("text/html", "html"), ("image/png", "png"), ("image/jpeg", "jpg"), ("application/pdf", "pdf"), ("text/plain", "txt"), ("application/xml", "xml"), ("text/csv", "csv"), ("application/zip", "zip"), ("audio/mpeg", "mp3"), ("video/mp4", "mp4")];
const DB_COLS: &[&str] = &["id", "title", "name", "email", "phone", "token", "group", "category", "password", "comment", "avatar", "status", "createdAt", "updatedAt"];
const DB_TYPES: &[&str] = &["int", "varchar", "text", "date", "datetime", "tinyint", "time", "timestamp", "smallint", "mediumint", "bigint", "decimal", "float", "double", "real", "bit", "boolean", "serial", "blob", "binary", "enum", "set", "geometry", "point"];
const DB_COLL: &[&str] = &["utf8_unicode_ci", "utf8_general_ci", "utf8_bin", "ascii_bin", "ascii_general_ci", "cp1250_bin", "cp1250_general_ci"];
const DB_ENG: &[&str] = &["InnoDB", "MyISAM", "MEMORY", "CSV", "BLACKHOLE", "ARCHIVE"];
const PREFIX: &[&str] = &["Mr.", "Mrs.", "Ms.", "Miss", "Dr."];
const SUFFIX: &[&str] = &["Jr.", "Sr.", "I", "II", "III", "IV", "V", "MD", "DDS", "PhD", "DVM"];
const PROTOCOLS: &[&str] = &["http", "https"];
const LOCALES: &[&str] = &["en", "hu", "de", "fr", "es", "it", "ja", "pl", "nl", "pt", "ru", "zh"];
const UAS: &[&str] = &[
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 13_4) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/16.5 Safari/605.1.15",
    "Mozilla/5.0 (X11; Linux x86_64; rv:121.0) Gecko/20100101 Firefox/121.0",
];
const FILE_EXT: &[&str] = &["pdf", "txt", "png", "jpg", "json", "html", "csv", "mp3", "mp4", "zip", "xml", "doc"];
const IMG_CATS: &[&str] = &["abstract", "animals", "business", "cats", "city", "food", "nightlife", "fashion", "people", "nature", "sports", "transport"];
const TRANS: &[&str] = &["deposit", "withdrawal", "payment", "invoice"];
const BS_ADJ: &[&str] = &["clicks-and-mortar", "value-added", "vertical", "proactive", "robust", "revolutionary", "scalable", "leading-edge", "innovative", "intuitive", "strategic", "e-business", "mission-critical", "sticky", "one-to-one", "24/7", "end-to-end", "global", "B2B", "B2C", "granular", "frictionless", "virtual", "viral", "dynamic"];
const BS_BUZZ: &[&str] = &["synergize", "strategize", "empower", "leverage", "envisioneer", "monetize", "harness", "facilitate", "seize", "disintermediate", "synthesize", "streamline", "transform", "deploy", "orchestrate", "maximize", "incentivize", "integrate"];
const BS_NOUN: &[&str] = &["synergies", "web-readiness", "paradigms", "markets", "partnerships", "infrastructures", "platforms", "initiatives", "channels", "eyeballs", "communities", "ROI", "solutions", "action-items", "portals", "niches", "technologies", "content", "supply-chains", "convergence", "relationships", "architectures", "interfaces", "e-markets", "e-commerce", "systems", "bandwidth", "models", "mindshare", "deliverables", "users", "schemas", "networks", "applications", "metrics", "functionalities", "experiences", "web services", "methodologies"];
const CP_ADJ: &[&str] = &["Adaptive", "Advanced", "Ameliorated", "Assimilated", "Automated", "Balanced", "Business-focused", "Centralized", "Cloned", "Compatible", "Configurable", "Cross-group", "Customer-focused", "Customizable", "Decentralized", "De-engineered", "Devolved", "Digitized", "Distributed", "Diverse", "Enhanced", "Ergonomic", "Exclusive", "Expanded", "Extended", "Face to face", "Focused", "Front-line", "Fully-configurable", "Function-based", "Fundamental", "Future-proofed", "Grass-roots", "Horizontal", "Implemented", "Innovative", "Integrated", "Intuitive", "Inverse", "Managed", "Mandatory", "Monitored", "Multi-channelled", "Multi-lateral", "Multi-layered", "Networked", "Object-based", "Open-architected", "Open-source", "Operative", "Optimized", "Optional", "Organic", "Organized", "Persevering", "Persistent", "Phased", "Polarised", "Pre-emptive", "Proactive", "Profit-focused", "Profound", "Programmable", "Progressive", "Public-key", "Quality-focused", "Reactive", "Realigned", "Re-contextualized", "Re-engineered", "Reduced", "Reverse-engineered", "Right-sized", "Robust", "Seamless", "Secured", "Self-enabling", "Sharable", "Stand-alone", "Streamlined", "Switchable", "Synchronised", "Synergistic", "Synergized", "Team-oriented", "Total", "Triple-buffered", "Universal", "Up-sized", "Upgradable", "User-centric", "User-friendly", "Versatile", "Virtual", "Visionary", "Vision-oriented"];
const CP_DESC: &[&str] = &["24 hour", "24/7", "3rd generation", "4th generation", "5th generation", "6th generation", "actuating", "analyzing", "asymmetric", "asynchronous", "attitude-oriented", "background", "bandwidth-monitored", "bi-directional", "bifurcated", "bottom-line", "clear-thinking", "client-driven", "client-server", "coherent", "cohesive", "composite", "context-sensitive", "contextually-based", "content-based", "dedicated", "demand-driven", "didactic", "directional", "discrete", "disintermediate", "dynamic", "eco-centric", "empowering", "encompassing", "even-keeled", "executive", "explicit", "exuding", "fault-tolerant", "foreground", "fresh-thinking", "full-range", "global", "grid-enabled", "heuristic", "high-level", "holistic", "homogeneous", "human-resource", "hybrid", "impactful", "incremental", "intangible", "interactive", "intermediate", "leading edge", "local", "logistical", "maximized", "methodical", "mission-critical", "mobile", "modular", "motivating", "multimedia", "multi-state", "multi-tasking", "national", "needs-based", "neutral", "next generation", "non-volatile", "object-oriented", "optimal", "optimizing", "radical", "real-time", "reciprocal", "regional", "responsive", "scalable", "secondary", "solution-oriented", "stable", "static", "systematic", "systemic", "system-worthy", "tangible", "tertiary", "transitional", "uniform", "upward-trending", "user-facing", "value-added", "web-enabled", "well-modulated", "zero administration", "zero defect", "zero tolerance"];
const CP_NOUN: &[&str] = &["ability", "access", "adapter", "algorithm", "alliance", "analyzer", "application", "approach", "architecture", "archive", "artificial intelligence", "array", "attitude", "benchmark", "budgetary management", "capability", "capacity", "challenge", "circuit", "collaboration", "complexity", "concept", "conglomeration", "contingency", "core", "customer loyalty", "database", "data-warehouse", "definition", "emulation", "encoding", "encryption", "extranet", "firmware", "flexibility", "focus group", "forecast", "frame", "framework", "function", "functionalities", "Graphic Interface", "groupware", "Graphical User Interface", "hardware", "help-desk", "hierarchy", "hub", "implementation", "info-mediaries", "infrastructure", "initiative", "installation", "instruction set", "interface", "internet solution", "intranet", "knowledge user", "knowledge base", "local area network", "leverage", "matrices", "matrix", "methodology", "middleware", "migration", "model", "moderator", "monitoring", "moratorium", "neural-net", "open architecture", "open system", "orchestration", "paradigm", "parallelism", "policy", "portal", "pricing structure", "process improvement", "product", "productivity", "project", "projection", "protocol", "secured line", "service-desk", "software", "solution", "standardization", "strategy", "structure", "success", "superstructure", "support", "synergy", "system engine", "task-force", "throughput", "time-frame", "toolset", "utilisation", "website", "workforce"];

fn pick<'a>(r: &mut impl Rng, a: &'a [&'a str]) -> &'a str {
    a.choose(r).copied().unwrap_or("")
}

fn lorem_words(r: &mut impl Rng, n: usize) -> String {
    (0..n).map(|_| pick(r, WORDS)).collect::<Vec<_>>().join(" ")
}

fn lorem_sentence(r: &mut impl Rng) -> String {
    let n = r.gen_range(4..10);
    let s = lorem_words(r, n);
    let mut c = s.chars();
    match c.next() {
        Some(f) => format!("{}{}.", f.to_uppercase(), c.as_str()),
        None => String::new(),
    }
}

fn lorem_paragraph(r: &mut impl Rng) -> String {
    (0..r.gen_range(3..6)).map(|_| lorem_sentence(r)).collect::<Vec<_>>().join(" ")
}

fn rand_date(r: &mut impl Rng, from_days: i64, to_days: i64) -> String {
    let now = chrono::Utc::now();
    let secs = r.gen_range(from_days * 86400..to_days * 86400);
    let d = now + chrono::Duration::seconds(secs);
    d.format("%a %b %d %Y %H:%M:%S GMT+0000 (Coordinated Universal Time)").to_string()
}

pub const DYNAMIC_NAMES: &[&str] = &[
    "$guid", "$timestamp", "$isoTimestamp", "$randomInt", "$randomUUID", "$randomAlphaNumeric", "$randomBoolean", "$randomAbbreviation", "$randomIP", "$randomIPV6", "$randomMACAddress", "$randomPassword", "$randomLocale", "$randomUserAgent", "$randomProtocol", "$randomSemver",
    "$randomFirstName", "$randomLastName", "$randomFullName", "$randomNamePrefix", "$randomNameSuffix", "$randomJobArea", "$randomJobDescriptor", "$randomJobTitle", "$randomJobType", "$randomPhoneNumber", "$randomPhoneNumberExt", "$randomCity", "$randomStreetName", "$randomStreetAddress", "$randomCountry", "$randomCountryCode", "$randomLatitude", "$randomLongitude",
    "$randomColor", "$randomHexColor", "$randomDepartment", "$randomProductName", "$randomPrice", "$randomProductAdjective", "$randomProductMaterial", "$randomProduct", "$randomCompanyName", "$randomCompanySuffix", "$randomCatchPhrase", "$randomBs", "$randomCatchPhraseAdjective", "$randomCatchPhraseDescriptor", "$randomCatchPhraseNoun", "$randomBsAdjective", "$randomBsBuzz", "$randomBsNoun",
    "$randomDatabaseColumn", "$randomDatabaseType", "$randomDatabaseCollation", "$randomDatabaseEngine", "$randomDateFuture", "$randomDatePast", "$randomDateRecent", "$randomWeekday", "$randomMonth",
    "$randomDomainName", "$randomDomainSuffix", "$randomDomainWord", "$randomEmail", "$randomExampleEmail", "$randomUserName", "$randomUrl",
    "$randomFileName", "$randomFileType", "$randomFileExt", "$randomCommonFileName", "$randomCommonFileType", "$randomCommonFileExt", "$randomFilePath", "$randomDirectoryPath", "$randomMimeType",
    "$randomBankAccount", "$randomBankAccountName", "$randomCreditCardMask", "$randomBankAccountBic", "$randomBankAccountIban", "$randomTransactionType", "$randomCurrencyCode", "$randomCurrencyName", "$randomCurrencySymbol", "$randomBitcoin",
    "$randomImageUrl", "$randomAvatarImage", "$randomAbstractImage", "$randomAnimalsImage", "$randomBusinessImage", "$randomCatsImage", "$randomCityImage", "$randomFoodImage", "$randomNightlifeImage", "$randomFashionImage", "$randomPeopleImage", "$randomNatureImage", "$randomSportsImage", "$randomTransportImage", "$randomImageDataUri",
    "$randomNoun", "$randomVerb", "$randomIngverb", "$randomAdjective", "$randomWord", "$randomWords", "$randomPhrase", "$randomLoremWord", "$randomLoremWords", "$randomLoremSentence", "$randomLoremSentences", "$randomLoremParagraph", "$randomLoremParagraphs", "$randomLoremText", "$randomLoremSlug", "$randomLoremLines",
];

pub fn dynamic_variable(name: &str) -> Option<String> {
    let r = &mut rand::thread_rng();
    let alnum = |r: &mut rand::rngs::ThreadRng, n: usize| -> String { (0..n).map(|_| r.sample(rand::distributions::Alphanumeric) as char).collect::<String>().to_lowercase() };
    let first = pick(r, FIRST).to_string();
    let last = pick(r, LAST).to_string();
    Some(match name {
        "$guid" | "$randomUUID" => uuid::Uuid::new_v4().to_string(),
        "$timestamp" => chrono::Utc::now().timestamp().to_string(),
        "$isoTimestamp" => chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        "$randomInt" => r.gen_range(0..=1000).to_string(),
        "$randomAlphaNumeric" => alnum(r, 1),
        "$randomBoolean" => r.gen_bool(0.5).to_string(),
        "$randomAbbreviation" => pick(r, &["TCP", "HTTP", "SDD", "RAM", "GB", "CSS", "SSL", "AGP", "SQL", "FTP", "PCI", "AI", "ADP", "RSS", "XML", "EXE", "COM", "HDD", "THX", "SMTP", "SMS", "USB", "PNG", "SAS", "IB", "SCSI", "JSON", "XSS", "JBOD"]).into(),
        "$randomIP" => format!("{}.{}.{}.{}", r.gen_range(1..255), r.gen::<u8>(), r.gen::<u8>(), r.gen_range(1..255)),
        "$randomIPV6" => (0..8).map(|_| format!("{:x}", r.gen::<u16>())).collect::<Vec<_>>().join(":"),
        "$randomMACAddress" => (0..6).map(|_| format!("{:02x}", r.gen::<u8>())).collect::<Vec<_>>().join(":"),
        "$randomPassword" => alnum(r, 15),
        "$randomLocale" => pick(r, LOCALES).into(),
        "$randomUserAgent" => pick(r, UAS).into(),
        "$randomProtocol" => pick(r, PROTOCOLS).into(),
        "$randomSemver" => format!("{}.{}.{}", r.gen_range(0..10), r.gen_range(0..10), r.gen_range(0..10)),
        "$randomFirstName" => first,
        "$randomLastName" => last,
        "$randomFullName" => format!("{first} {last}"),
        "$randomNamePrefix" => pick(r, PREFIX).into(),
        "$randomNameSuffix" => pick(r, SUFFIX).into(),
        "$randomJobArea" => pick(r, JOB_AREA).into(),
        "$randomJobDescriptor" => pick(r, JOB_DESC).into(),
        "$randomJobType" => pick(r, JOB_TYPE).into(),
        "$randomJobTitle" => format!("{} {} {}", pick(r, JOB_DESC), pick(r, JOB_AREA), pick(r, JOB_TYPE)),
        "$randomPhoneNumber" => format!("{}-{}-{}", r.gen_range(200..999), r.gen_range(200..999), r.gen_range(1000..9999)),
        "$randomPhoneNumberExt" => format!("{}-{}-{}-{}", r.gen_range(1..99), r.gen_range(200..999), r.gen_range(200..999), r.gen_range(1000..9999)),
        "$randomCity" => pick(r, CITIES).into(),
        "$randomStreetName" => pick(r, STREETS).into(),
        "$randomStreetAddress" => format!("{} {}", r.gen_range(1..9999), pick(r, STREETS)),
        "$randomCountry" => COUNTRIES.choose(r).unwrap().0.into(),
        "$randomCountryCode" => COUNTRIES.choose(r).unwrap().1.into(),
        "$randomLatitude" => format!("{:.4}", r.gen_range(-90.0..90.0f64)),
        "$randomLongitude" => format!("{:.4}", r.gen_range(-180.0..180.0f64)),
        "$randomColor" => pick(r, COLORS).into(),
        "$randomHexColor" => format!("#{:06x}", r.gen_range(0..0xffffffu32)),
        "$randomDepartment" => pick(r, DEPTS).into(),
        "$randomProductName" => format!("{} {} {}", pick(r, PADJ), pick(r, MATERIALS), pick(r, PRODUCTS)),
        "$randomPrice" => format!("{:.2}", r.gen_range(1.0..1000.0f64)),
        "$randomProductAdjective" => pick(r, PADJ).into(),
        "$randomProductMaterial" => pick(r, MATERIALS).into(),
        "$randomProduct" => pick(r, PRODUCTS).into(),
        "$randomCompanyName" => format!("{} {}", last, pick(r, COMPANY_SUFFIX)),
        "$randomCompanySuffix" => pick(r, COMPANY_SUFFIX).into(),
        "$randomCatchPhrase" => format!("{} {} {}", pick(r, CP_ADJ), pick(r, CP_DESC), pick(r, CP_NOUN)),
        "$randomCatchPhraseAdjective" => pick(r, CP_ADJ).into(),
        "$randomCatchPhraseDescriptor" => pick(r, CP_DESC).into(),
        "$randomCatchPhraseNoun" => pick(r, CP_NOUN).into(),
        "$randomBs" => format!("{} {} {}", pick(r, BS_BUZZ), pick(r, BS_ADJ), pick(r, BS_NOUN)),
        "$randomBsAdjective" => pick(r, BS_ADJ).into(),
        "$randomBsBuzz" => pick(r, BS_BUZZ).into(),
        "$randomBsNoun" => pick(r, BS_NOUN).into(),
        "$randomDatabaseColumn" => pick(r, DB_COLS).into(),
        "$randomDatabaseType" => pick(r, DB_TYPES).into(),
        "$randomDatabaseCollation" => pick(r, DB_COLL).into(),
        "$randomDatabaseEngine" => pick(r, DB_ENG).into(),
        "$randomDateFuture" => rand_date(r, 1, 365),
        "$randomDatePast" => rand_date(r, -365, -1),
        "$randomDateRecent" => rand_date(r, -2, 0),
        "$randomWeekday" => pick(r, WEEKDAYS).into(),
        "$randomMonth" => pick(r, MONTHS).into(),
        "$randomDomainWord" => format!("{}-{}", first.to_lowercase(), pick(r, WORDS)),
        "$randomDomainSuffix" => pick(r, DOMAIN_SUFFIX).into(),
        "$randomDomainName" => format!("{}.{}", last.to_lowercase(), pick(r, DOMAIN_SUFFIX)),
        "$randomEmail" => format!("{}.{}@{}.{}", first, last, pick(r, &["gmail", "yahoo", "hotmail"]), "com"),
        "$randomExampleEmail" => format!("{}{}@example.{}", first, r.gen_range(1..99), pick(r, &["com", "net", "org"])),
        "$randomUserName" => format!("{}.{}{}", first, last, r.gen_range(1..99)),
        "$randomUrl" => format!("https://{}.{}", last.to_lowercase(), pick(r, DOMAIN_SUFFIX)),
        "$randomFileName" | "$randomCommonFileName" => format!("{}_{}.{}", pick(r, WORDS), pick(r, WORDS), pick(r, FILE_EXT)),
        "$randomFileType" | "$randomCommonFileType" => pick(r, &["application", "audio", "image", "text", "video"]).into(),
        "$randomFileExt" | "$randomCommonFileExt" => pick(r, FILE_EXT).into(),
        "$randomDirectoryPath" => format!("/{}/{}", pick(r, &["usr", "var", "home", "opt", "etc"]), pick(r, WORDS)),
        "$randomFilePath" => format!("/{}/{}/{}.{}", pick(r, &["usr", "var", "home", "opt"]), pick(r, WORDS), pick(r, WORDS), pick(r, FILE_EXT)),
        "$randomMimeType" => MIME.choose(r).unwrap().0.into(),
        "$randomBankAccount" => format!("{:08}", r.gen_range(0..100_000_000u32)),
        "$randomBankAccountName" => pick(r, &["Home Loan Account", "Checking Account", "Savings Account", "Money Market Account", "Credit Card Account", "Investment Account", "Personal Loan Account", "Auto Loan Account"]).into(),
        "$randomCreditCardMask" => format!("{:04}", r.gen_range(0..10000)),
        "$randomBankAccountBic" => format!("{}{}{}", alnum(r, 4).to_uppercase(), pick(r, &["HU", "DE", "US", "GB"]), alnum(r, 2).to_uppercase()),
        "$randomBankAccountIban" => format!("HU{:02}{:024}", r.gen_range(10..99), r.gen_range(0..u64::MAX) as u128 * 1000),
        "$randomTransactionType" => pick(r, TRANS).into(),
        "$randomCurrencyCode" => CURRENCIES.choose(r).unwrap().0.into(),
        "$randomCurrencyName" => CURRENCIES.choose(r).unwrap().1.into(),
        "$randomCurrencySymbol" => CURRENCIES.choose(r).unwrap().2.into(),
        "$randomBitcoin" => format!("1{}", (0..33).map(|_| r.sample(rand::distributions::Alphanumeric) as char).collect::<String>()),
        "$randomImageUrl" | "$randomAvatarImage" => format!("https://picsum.photos/seed/{}/640/480", r.gen::<u32>()),
        "$randomImageDataUri" => "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==".into(),
        n if n.starts_with("$random") && n.ends_with("Image") => {
            let cat = n.trim_start_matches("$random").trim_end_matches("Image").to_lowercase();
            if IMG_CATS.contains(&cat.as_str()) {
                format!("https://loremflickr.com/640/480/{cat}")
            } else {
                return None;
            }
        }
        "$randomNoun" => pick(r, NOUNS).into(),
        "$randomVerb" => pick(r, VERBS).into(),
        "$randomIngverb" => pick(r, INGVERBS).into(),
        "$randomAdjective" => pick(r, ADJ).into(),
        "$randomWord" | "$randomLoremWord" => pick(r, WORDS).into(),
        "$randomWords" | "$randomLoremWords" => lorem_words(r, 3),
        "$randomPhrase" => format!("Try to {} the {} {}, maybe it will {} the {} {}!", pick(r, VERBS), pick(r, ADJ), pick(r, NOUNS), pick(r, VERBS), pick(r, ADJ), pick(r, NOUNS)),
        "$randomLoremSentence" => lorem_sentence(r),
        "$randomLoremSentences" => (0..r.gen_range(2..6)).map(|_| lorem_sentence(r)).collect::<Vec<_>>().join(" "),
        "$randomLoremParagraph" => lorem_paragraph(r),
        "$randomLoremParagraphs" => (0..3).map(|_| lorem_paragraph(r)).collect::<Vec<_>>().join("\n \r"),
        "$randomLoremText" => lorem_paragraph(r),
        "$randomLoremSlug" => lorem_words(r, 3).replace(' ', "-"),
        "$randomLoremLines" => (0..r.gen_range(1..5)).map(|_| lorem_sentence(r)).collect::<Vec<_>>().join("\n"),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nested() {
        let mut c = VarContext::default();
        c.environment.set("host", Value::String("{{proto}}://x.com".into()));
        c.globals.set("proto", Value::String("https".into()));
        c.collection.set("host", Value::String("nope".into()));
        assert_eq!(c.resolve("{{host}}/a/{{missing}}"), "https://x.com/a/{{missing}}");
        assert_eq!(c.resolve("{{$guid}}").len(), 36);
        assert!(c.resolve("{{$randomInt}}").parse::<u32>().is_ok());
    }
}
