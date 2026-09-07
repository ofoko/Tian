//! tc —— Tian Compiler 天语言编译器
//!
//! 阶段一：C 中转后端（tc xxx.t --emit-c / tc run xxx.t）
//! 阶段二：LLVM 后端（tc build xxx.t，规划中）

mod ast;
mod fmt;
mod gen_c;
mod gen_native;
mod lexer;
mod parser;
mod type_check;

use std::fs;
use std::path::PathBuf;
use std::process::exit;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }

    match args[1].as_str() {
        "lex" => {
            if args.len() < 3 {
                usage();
            }
            let src = read_source(&args[2]);
            match lexer::lex(&src) {
                Ok(toks) => {
                    for t in &toks {
                        println!("{}", t);
                    }
                    println!("共 {} 个 Token，词法分析通过", toks.len());
                }
                Err(e) => {
                    eprintln!("{}", e);
                    exit(1);
                }
            }
        }
        "run" => {
            if args.len() < 3 {
                usage();
            }
            cmd_run(&args[2]);
        }
        "test" => {
            if args.len() < 3 {
                usage();
            }
            cmd_test(&args[2]);
        }
        "build" => {
            if args.len() < 3 {
                usage();
            }
            cmd_build(&args[2]);
        }
        "fmt" => {
            if args.len() < 3 {
                usage();
            }
            cmd_fmt(&args[2]);
        }
        "--emit-c" => {
            eprintln!("错误：--emit-c 需放在源文件之后，用法：tc xxx.t --emit-c");
            exit(1);
        }
        file => {
            // tc xxx.t --emit-c / tc xxx.t --diagnose-json
            match args.get(2).map(|s| s.as_str()) {
                Some("--emit-c") => cmd_emit_c(file),
                Some("--diagnose-json") => cmd_diagnose_json(file),
                _ => usage(),
            }
        }
    }
}

/// 完整编译管线：源文件 → Token → AST → 类型检查 → C 源码
fn compile(path: &str) -> Result<String, String> {
    let prog = parser::parse_file(path).map_err(|e| e.to_string())?;
    type_check::check(&prog).map_err(|e| e.to_string())?;
    Ok(gen_c::generate(&prog))
}

/// v2.3 差分诊断：结构化 JSON 输出（供 AI/工具链程序化读取，
/// 只替换 error_line 处的 wrong_token，杜绝"越修越错"）
fn cmd_diagnose_json(path: &str) {
    let json = |stage: &str, line: usize, msg: &str, hint: &str, candidates: &[String]| {
        let cands: Vec<String> = candidates
            .iter()
            .map(|c| format!("\"{}\"", json_escape(c)))
            .collect();
        format!(
            "{{\"stage\":\"{}\",\"error\":{},\"line\":{},\"message\":\"{}\",\"hint\":\"{}\",\"candidates\":[{}]}}",
            stage,
            !msg.is_empty() && stage != "ok",
            line,
            json_escape(msg),
            json_escape(hint),
            cands.join(",")
        )
    };

    let prog = match parser::parse_file(path) {
        Ok(p) => p,
        Err(e) => {
            let is_lex = e.msg.starts_with("词法错误");
            println!(
                "{}",
                json(if is_lex { "lex" } else { "parse" }, e.line, &e.msg, "检查语句是否以换行结束、块是否用 } 闭合", &[])
            );
            exit(1);
        }
    };
    if let Err(e) = type_check::check(&prog) {
        println!(
            "{}",
            json("check", e.line, &e.msg, &e.hint, &e.candidates)
        );
        exit(1);
    }
    println!(
        "{}",
        json("ok", 0, "", "编译通过：类型检查与天权所有权检查均无错误", &[])
    );
}

/// JSON 字符串转义（最小集：引号、反斜杠、控制字符）
fn json_escape(s: &str) -> String {
    let mut r = String::new();
    for c in s.chars() {
        match c {
            '"' => r.push_str("\\\""),
            '\\' => r.push_str("\\\\"),
            '\n' => r.push_str("\\n"),
            '\r' => r.push_str("\\r"),
            '\t' => r.push_str("\\t"),
            c if (c as u32) < 0x20 => r.push_str(&format!("\\u{:04x}", c as u32)),
            c => r.push(c),
        }
    }
    r
}

/// tc xxx.t --emit-c
fn cmd_emit_c(path: &str) {    match compile(path) {
        Ok(c) => {
            let out = out_path(path, "c");
            fs::write(&out, c).unwrap_or_else(|e| {
                eprintln!("无法写入 {}：{}", out.display(), e);
                exit(1);
            });
            println!("已生成 {}", out.display());
        }
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    }
}

fn cmd_run(path: &str) {
    let c = match compile(path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    };
    let stem = out_path(path, "").file_name().unwrap().to_string_lossy().to_string();
    compile_and_run_c(c, stem);
}

/// v2.2 语义锚点：tc test —— 只运行 @example 自检（不执行顶层语句），
/// 全部通过静默退出 0，任一失败非零退出（规范第 12 节）
fn cmd_test(path: &str) {
    let mut prog = match parser::parse_file(path).map_err(|e| e.to_string()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    };
    if let Err(e) = type_check::check(&prog) {
        eprintln!("{}", e);
        exit(1);
    }
    prog.top.clear();
    let c = gen_c::generate(&prog);
    let stem = out_path(path, "").file_name().unwrap().to_string_lossy().to_string();
    compile_and_run_c(c, stem);
}

/// 将生成的 C 源码编译为可执行文件并运行（tc run / tc test 共用）
fn compile_and_run_c(c: String, stem: String) {
    let tmp = std::env::temp_dir();
    let c_path = tmp.join(format!("tian_{}.c", stem));
    let bin_path = tmp.join(format!("tian_{}", stem));
    fs::write(&c_path, &c).unwrap_or_else(|e| {
        eprintln!("无法写入临时文件 {}：{}", c_path.display(), e);
        exit(1);
    });

    // 优先 cc（macOS 自带），回退 gcc
    let cc = ["cc", "gcc"]
        .into_iter()
        .find(|c| std::process::Command::new(c).arg("--version").output().is_ok())
        .unwrap_or_else(|| {
            eprintln!("未找到 C 编译器（cc/gcc），tc run 依赖它编译生成的 C 代码");
            exit(1);
        });

    let status = std::process::Command::new(cc)
        .arg("-O2")
        .arg("-w")
        .arg(&c_path)
        .arg("-o")
        .arg(&bin_path)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("调用 {} 失败：{}", cc, e);
            exit(1);
        });
    if !status.success() {
        eprintln!("C 编译失败（生成的 C 代码：{}）", c_path.display());
        exit(1);
    }
    let status = std::process::Command::new(&bin_path).status().unwrap_or_else(|e| {
        eprintln!("运行失败：{}", e);
        exit(1);
    });
    exit(status.code().unwrap_or(1));
}

/// tc build：Cranelift 原生后端（v1.1）→ .o + 运行时链接；失败回退 C 后端
/// tc fmt：规范化源码（v3.9，规范第 20 节）——统一缩进与运算符空格，
/// 顶层顺序规范化为 use → struct → fn → 顶层语句；输出到 stdout
fn cmd_fmt(path: &str) {
    let prog = match parser::parse_file(path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    };
    if let Err(e) = type_check::check(&prog) {
        eprintln!("{}", e);
        exit(1);
    }
    print!("{}", fmt::format(&prog));
}

fn cmd_build(path: &str) {
    let prog = match parser::parse_file(path).map_err(|e| e.to_string()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", e);
            exit(1);
        }
    };
    if let Err(e) = type_check::check(&prog) {
        eprintln!("{}", e);
        exit(1);
    }

    let bin_path = out_path(path, "");
    let tmp = std::env::temp_dir();
    let stem = bin_path.file_name().unwrap().to_string_lossy().to_string();

    // 主路径：Cranelift 原生机器码（不再经 C 中转）
    match gen_native::generate_object(&prog) {
        Ok(obj) => {
            let o_path = tmp.join(format!("tian_{}.o", stem));
            let rt_c = tmp.join("tian_rt.c");
            let rt_o = tmp.join("tian_rt.o");
            fs::write(&o_path, obj).unwrap_or_else(|e| {
                eprintln!("无法写入 {}：{}", o_path.display(), e);
                exit(1);
            });
            fs::write(&rt_c, include_str!("tian_rt.c")).unwrap_or_else(|e| {
                eprintln!("无法写入 {}：{}", rt_c.display(), e);
                exit(1);
            });
            let cc = find_cc();
            let st = std::process::Command::new(cc).arg("-O2").arg("-c").arg(&rt_c).arg("-o").arg(&rt_o)
                .status().unwrap_or_else(|e| { eprintln!("调用 {} 失败：{}", cc, e); exit(1); });
            if !st.success() {
                eprintln!("运行时编译失败");
                exit(1);
            }
            let st = std::process::Command::new(cc).arg(&rt_o).arg(&o_path).arg("-o").arg(&bin_path)
                .status().unwrap_or_else(|e| { eprintln!("调用 {} 失败：{}", cc, e); exit(1); });
            if !st.success() {
                eprintln!("原生链接失败（目标文件：{}）", o_path.display());
                exit(1);
            }
            println!("已生成原生可执行文件 {}（Cranelift 机器码）", bin_path.display());
        }
        Err(e) => {
            // 兜底：C 后端保证可用性；同时报出原生后端错误供诊断
            eprintln!("原生后端失败（{}），回退 C 后端", e);
            let c = gen_c::generate(&prog);
            let c_path = tmp.join(format!("tian_{}.c", stem));
            fs::write(&c_path, &c).unwrap_or_else(|e| {
                eprintln!("无法写入临时文件 {}：{}", c_path.display(), e);
                exit(1);
            });
            let cc = find_cc();
            let st = std::process::Command::new(cc).arg("-O2").arg("-w").arg(&c_path).arg("-o").arg(&bin_path)
                .status().unwrap_or_else(|e| { eprintln!("调用 {} 失败：{}", cc, e); exit(1); });
            if !st.success() {
                eprintln!("C 编译失败（生成的 C 代码：{}）", c_path.display());
                exit(1);
            }
            println!("已生成可执行文件 {}（C 后端回退）", bin_path.display());
        }
    }
}

fn find_cc() -> &'static str {
    ["cc", "gcc"]
        .into_iter()
        .find(|c| std::process::Command::new(c).arg("--version").output().is_ok())
        .unwrap_or_else(|| {
            eprintln!("未找到 C 编译器（cc/gcc），链接阶段依赖它");
            exit(1);
        })
}

/// xxx.t → xxx.<ext>
fn out_path(path: &str, ext: &str) -> PathBuf {
    let mut p = PathBuf::from(path);
    p.set_extension(ext);
    p
}

fn read_source(path: &str) -> String {
    match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("无法读取源文件 {}：{}", path, e);
            exit(1);
        }
    }
}

fn usage() {
    eprintln!("tc —— 天语言编译器 v3.7（Cranelift 原生后端 + C 中转后端）");
    eprintln!();
    eprintln!("用法：");
    eprintln!("  tc lex xxx.t        词法分析（调试，打印 Token 流）");
    eprintln!("  tc xxx.t --emit-c   编译为中间 C 代码，输出 xxx.c");
    eprintln!("  tc xxx.t --diagnose-json  结构化差分诊断（JSON，供工具链读取）");
    eprintln!("  tc run xxx.t        编译并运行，一步到位");
    eprintln!("  tc test xxx.t       只运行 @example 语义锚点自检（v2.2）");
    eprintln!("  tc build xxx.t      原生机器码编译（Cranelift，失败回退 C 后端）");
    eprintln!("  tc fmt xxx.t        规范化源码格式（v3.9，输出到 stdout）");
    exit(1);
}
