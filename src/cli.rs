//! newman-compatible command line runner: `postmini run <collection> [options]`

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::model::{parse_collection, Environment};
use crate::runner::{self, RunEvent, RunOptions};
use crate::settings::Settings;
use crate::vars::{Scope, VarContext};

pub const HELP: &str = r#"postmini - lightweight Postman-compatible API client

USAGE:
  postmini                         start the graphical app
  postmini run <collection> [options]   run a collection (newman compatible)

RUN OPTIONS:
  -e, --environment <file>         environment JSON file
  -g, --globals <file>             globals JSON file
  -n, --iteration-count <n>        number of iterations
  -d, --iteration-data <file>      CSV or JSON data file
      --folder <name>              run only the given folder
      --env-var "key=value"        set environment variable (repeatable)
      --global-var "key=value"     set global variable (repeatable)
      --delay-request <ms>         delay between requests
      --timeout-request <ms>       request timeout
      --timeout-script <ms>        script timeout
  -k, --insecure                   disable SSL verification
      --ignore-redirects           do not follow redirects
      --bail                       stop on first error / failed test
      --silent                     no console output
      --verbose                    print request/response details
      --export-environment <file>  write final environment
      --export-globals <file>      write final globals
      --export-collection <file>   write collection with final collection variables
      --reporter-json-export <file>   JSON summary report
      --reporter-junit-export <file>  JUnit XML report
      --working-dir <dir>          base directory for file uploads
      --ssl-client-cert <file> --ssl-client-key <file>
      --ssl-extra-ca-certs <file>
      --proxy <url>                proxy URL (http/socks5)
  -h, --help                       show this help
  -v, --version                    show version
"#;

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("Cannot read {path}: {e}"))
}

fn xml_esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub fn main(args: Vec<String>) -> i32 {
    if args.is_empty() || args[0] == "-h" || args[0] == "--help" || args[0] == "help" {
        println!("{HELP}");
        return 0;
    }
    if args[0] == "-v" || args[0] == "--version" {
        println!("postmini {}", env!("CARGO_PKG_VERSION"));
        return 0;
    }
    if args[0] != "run" {
        eprintln!("Unknown command: {}\n\n{HELP}", args[0]);
        return 2;
    }
    match run(&args[1..]) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<i32, String> {
    let mut coll_path = None;
    let mut env_path = None;
    let mut globals_path = None;
    let mut iterations = 0usize;
    let mut data_path = None;
    let mut folder = None;
    let mut env_vars = vec![];
    let mut global_vars = vec![];
    let mut settings = Settings::default();
    settings.proxy_mode = "system".into();
    let mut bail = false;
    let mut silent = false;
    let mut verbose = false;
    let mut delay = 0u64;
    let mut export_env = None;
    let mut export_globals = None;
    let mut export_coll = None;
    let mut json_export = None;
    let mut junit_export = None;
    let mut cert = None;
    let mut key = None;
    let mut i = 0;
    let val = |i: &mut usize| -> Result<String, String> {
        *i += 1;
        args.get(*i).cloned().ok_or_else(|| format!("Missing value for {}", args[*i - 1]))
    };
    while i < args.len() {
        let a = args[i].as_str();
        match a {
            "-e" | "--environment" => env_path = Some(val(&mut i)?),
            "-g" | "--globals" => globals_path = Some(val(&mut i)?),
            "-n" | "--iteration-count" => iterations = val(&mut i)?.parse().map_err(|_| "Invalid iteration count")?,
            "-d" | "--iteration-data" => data_path = Some(val(&mut i)?),
            "--folder" => folder = Some(val(&mut i)?),
            "--env-var" => env_vars.push(val(&mut i)?),
            "--global-var" => global_vars.push(val(&mut i)?),
            "--delay-request" => delay = val(&mut i)?.parse().unwrap_or(0),
            "--timeout-request" => settings.timeout_ms = val(&mut i)?.parse().unwrap_or(0),
            "--timeout-script" => settings.script_timeout_ms = val(&mut i)?.parse().unwrap_or(10_000),
            "--timeout" => {
                let _ = val(&mut i)?;
            }
            "-k" | "--insecure" => settings.verify_ssl = false,
            "--ignore-redirects" => settings.follow_redirects = false,
            "--bail" => bail = true,
            "--silent" => silent = true,
            "--verbose" => verbose = true,
            "--export-environment" => export_env = Some(val(&mut i)?),
            "--export-globals" => export_globals = Some(val(&mut i)?),
            "--export-collection" => export_coll = Some(val(&mut i)?),
            "--reporter-json-export" => json_export = Some(val(&mut i)?),
            "--reporter-junit-export" => junit_export = Some(val(&mut i)?),
            "-r" | "--reporters" => {
                let _ = val(&mut i)?;
            }
            "--working-dir" => settings.working_dir = val(&mut i)?,
            "--ssl-client-cert" => cert = Some(val(&mut i)?),
            "--ssl-client-key" => key = Some(val(&mut i)?),
            "--ssl-extra-ca-certs" => settings.ca_file = val(&mut i)?,
            "--proxy" => {
                settings.proxy_mode = "custom".into();
                settings.proxy.url = val(&mut i)?;
            }
            "--color" | "--disable-unicode" | "--no-color" | "--suppress-exit-code" | "-x" => {
                if a == "--color" {
                    let _ = val(&mut i)?;
                }
            }
            _ if a.starts_with('-') => return Err(format!("Unknown option {a}")),
            _ => coll_path = Some(a.to_string()),
        }
        i += 1;
    }
    if let Some(c) = cert {
        settings.client_certs.push(crate::http::ClientCert { host: "*".into(), cert_path: c, key_path: key.unwrap_or_default(), enabled: true });
    }
    let coll_path = coll_path.ok_or("No collection given")?;
    let mut coll = parse_collection(&read(&coll_path)?)?;
    let mut env = match &env_path {
        Some(p) => Environment::parse(&read(p)?)?,
        None => Environment::new("CLI"),
    };
    let mut globals = match &globals_path {
        Some(p) => Environment::parse(&read(p)?)?,
        None => Environment::new("Globals"),
    };
    for kv in &env_vars {
        if let Some((k, v)) = kv.split_once('=') {
            env.values.retain(|x| x.key != k);
            env.values.push(crate::model::EnvVar::new(k, v));
        }
    }
    for kv in &global_vars {
        if let Some((k, v)) = kv.split_once('=') {
            globals.values.retain(|x| x.key != k);
            globals.values.push(crate::model::EnvVar::new(k, v));
        }
    }
    let data = match &data_path {
        Some(p) => runner::parse_data_file(p)?,
        None => vec![],
    };
    let folder_path = match &folder {
        Some(name) => {
            fn find(items: &[crate::model::Item], name: &str, pre: &mut Vec<usize>) -> bool {
                for (i, it) in items.iter().enumerate() {
                    pre.push(i);
                    if it.is_folder() && it.name == name {
                        return true;
                    }
                    if let Some(ch) = &it.item {
                        if find(ch, name, pre) {
                            return true;
                        }
                    }
                    pre.pop();
                }
                false
            }
            let mut p = vec![];
            if !find(&coll.item, name, &mut p) {
                return Err(format!("Folder \"{name}\" not found"));
            }
            Some(p)
        }
        None => None,
    };
    let vars = VarContext { environment: Scope::from_env(&env), env_name: env.name.clone(), globals: Scope::from_env(&globals), collection: Scope::from_vars(&coll.variable), ..Default::default() };
    let opts = RunOptions { iterations, delay_ms: delay, data, bail, selected: None, folder: folder_path };
    let jar = Some(crate::http::new_jar());
    if !silent {
        println!("postmini\n\n{}", coll.info.name);
    }
    let mut final_vars = None;
    let total_iter = if iterations == 0 { opts.data.len().max(1) } else { iterations };
    let summary = runner::run_collection(&coll, vars, &opts, &settings, jar, Arc::new(AtomicBool::new(false)), |ev| match ev {
        RunEvent::IterationStart(i) => {
            if !silent && total_iter > 1 {
                println!("\nIteration {}/{}", i + 1, total_iter);
            }
        }
        RunEvent::Item(r, console) => {
            if silent {
                return;
            }
            println!("\n→ {}", r.name);
            for c in &console {
                if c.level != "net" || verbose {
                    let pre = match c.level.as_str() {
                        "net" => "  ",
                        "error" => "  ┊ ✖ ",
                        "warn" => "  ┊ ⚠ ",
                        _ => "  ┊ ",
                    };
                    for line in c.text.lines() {
                        println!("{pre}{line}");
                    }
                }
            }
            if r.status > 0 {
                println!("  {} {} [{} {}, {}, {:.0}ms]", r.method, r.url, r.status, r.reason, runner::human_size(r.size), r.time_ms);
            } else if r.skipped {
                println!("  (skipped)");
            }
            if let Some(e) = &r.error {
                println!("  ✖ {e}");
            }
            let mut n = 0;
            for t in &r.tests {
                if t.skipped {
                    println!("  - {} (skipped)", t.name);
                } else if t.passed {
                    println!("  ✓  {}", t.name);
                } else {
                    n += 1;
                    println!("  {n}. {}\n     {}", t.name, t.error.clone().unwrap_or_default());
                }
            }
        }
        RunEvent::Done(_, v) => final_vars = Some(v),
    });
    if !silent {
        let total_assert = summary.tests_passed + summary.tests_failed;
        println!();
        println!("┌─────────────────────────┬────────────┬────────────┐");
        println!("│                         │   executed │     failed │");
        println!("├─────────────────────────┼────────────┼────────────┤");
        println!("│              iterations │ {:>10} │ {:>10} │", summary.iterations, 0);
        println!("│                requests │ {:>10} │ {:>10} │", summary.requests, summary.failed_requests);
        println!("│              assertions │ {:>10} │ {:>10} │", total_assert, summary.tests_failed);
        println!("├─────────────────────────┴────────────┴────────────┤");
        println!("│ total run duration: {:<30}│", format!("{:.1}s", summary.total_time_ms / 1000.0));
        println!("│ average response time: {:<27}│", format!("{:.0}ms", summary.avg_response_ms));
        println!("└──────────────────────────────────────────────────┘");
        let failures: Vec<_> = summary.results.iter().flat_map(|r| r.tests.iter().filter(|t| !t.passed && !t.skipped).map(move |t| (r, t))).collect();
        if !failures.is_empty() {
            println!("\n  #  failure         detail\n");
            for (i, (r, t)) in failures.iter().enumerate() {
                println!(" {:>2}. AssertionError  {}\n                    {}\n                    at assertion in \"{}\"\n", i + 1, t.name, t.error.clone().unwrap_or_default(), r.name);
            }
        }
    }
    if let Some(v) = &final_vars {
        if let Some(p) = export_env {
            v.environment.write_env(&mut env);
            std::fs::write(&p, serde_json::to_string_pretty(&env).unwrap()).map_err(|e| e.to_string())?;
        }
        if let Some(p) = export_globals {
            v.globals.write_env(&mut globals);
            globals.scope = "globals".into();
            std::fs::write(&p, serde_json::to_string_pretty(&globals).unwrap()).map_err(|e| e.to_string())?;
        }
        if let Some(p) = export_coll {
            v.collection.write_vars(&mut coll.variable);
            std::fs::write(&p, serde_json::to_string_pretty(&coll).unwrap()).map_err(|e| e.to_string())?;
        }
    }
    if let Some(p) = json_export {
        let j = serde_json::json!({ "collection": {"name": coll.info.name, "id": coll.id()}, "run": summary });
        std::fs::write(&p, serde_json::to_string_pretty(&j).unwrap()).map_err(|e| e.to_string())?;
    }
    if let Some(p) = junit_export {
        let mut x = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        let tests: usize = summary.results.iter().map(|r| r.tests.len().max(1)).sum();
        x.push_str(&format!("<testsuites name=\"{}\" tests=\"{}\" time=\"{:.3}\">\n", xml_esc(&coll.info.name), tests, summary.total_time_ms / 1000.0));
        for r in &summary.results {
            let failures = r.tests.iter().filter(|t| !t.passed && !t.skipped).count() + usize::from(r.error.is_some() && r.tests.is_empty());
            x.push_str(&format!("  <testsuite name=\"{}\" id=\"{}\" tests=\"{}\" failures=\"{}\" errors=\"0\" time=\"{:.3}\">\n", xml_esc(&r.name), xml_esc(&r.id), r.tests.len().max(1), failures, r.time_ms / 1000.0));
            if r.tests.is_empty() {
                x.push_str(&format!("    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">", xml_esc(&format!("{} {}", r.method, r.url)), xml_esc(&coll.info.name), r.time_ms / 1000.0));
                if let Some(e) = &r.error {
                    x.push_str(&format!("<failure type=\"Error\" message=\"{}\"/>", xml_esc(e)));
                }
                x.push_str("</testcase>\n");
            }
            for t in &r.tests {
                x.push_str(&format!("    <testcase name=\"{}\" classname=\"{}\" time=\"{:.3}\">", xml_esc(&t.name), xml_esc(&r.name), r.time_ms / 1000.0));
                if t.skipped {
                    x.push_str("<skipped/>");
                } else if !t.passed {
                    x.push_str(&format!("<failure type=\"AssertionFailure\" message=\"{}\"><![CDATA[{}]]></failure>", xml_esc(t.error.as_deref().unwrap_or("")), t.error.as_deref().unwrap_or("")));
                }
                x.push_str("</testcase>\n");
            }
            x.push_str("  </testsuite>\n");
        }
        x.push_str("</testsuites>\n");
        std::fs::write(&p, x).map_err(|e| e.to_string())?;
    }
    Ok(if summary.tests_failed > 0 || summary.failed_requests > 0 || summary.results.iter().any(|r| r.error.is_some()) { 1 } else { 0 })
}
