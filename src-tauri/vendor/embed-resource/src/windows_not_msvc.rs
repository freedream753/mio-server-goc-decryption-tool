use self::super::apply_macros;
use std::process::Command;
use std::path::PathBuf;
use std::borrow::Cow;
use std::ffi::OsStr;
use std::env;


#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct ResourceCompiler;


impl ResourceCompiler {
    #[inline(always)]
    pub fn new() -> ResourceCompiler {
        ResourceCompiler
    }

    #[inline(always)]
    pub fn is_supported(&self) -> bool {
        true
    }

    pub fn compile_resource<Ms: AsRef<OsStr>, Mi: IntoIterator<Item = Ms>>(&self, out_dir: &str, prefix: &str, resource: &str, macros: Mi) -> String {
        let out_file = format!("{}/lib{}.a", out_dir, prefix);

        // Under some msys2 environments, $MINGW_CHOST has the correct target for
        // GNU windres or llvm-windres (clang32, clang64, or clangarm64)
        let target = env::var_os("MINGW_CHOST").map(Cow::Owned).unwrap_or_else(|| {
            OsStr::new(match env::var("TARGET").expect("No TARGET env var").as_bytes() {
                    [b'x', b'8', b'6', b'_', b'6', b'4', ..] => "pe-x86-64", // "x86_64"
                    // 本地修改：llvm-windres 只认 triple 形式的 aarch64 目标（pe-aarch64-little 会报
                    // Unsupported architecture），配合 llvm-mingw 工具链使用
                    [b'a', b'a', b'r', b'c', b'h', b'6', b'4', ..] => "aarch64-w64-mingw32",
                    _ => "pe-i386",
                })
                .into()
        });

        match apply_macros(Command::new("windres")
                               .args(&["--input", resource, "--output-format=coff", "--target"])
                               .arg(target)
                               .args(&["--output", &out_file, "--include-dir", out_dir]),
                           "-D",
                           macros)
            .status() {
            Ok(stat) if stat.success() => {}
            Ok(stat) => panic!("windres failed to compile \"{}\" into \"{}\" with {}", resource, out_file, stat),
            Err(e) => panic!("Couldn't to execute windres to compile \"{}\" into \"{}\": {}", resource, out_file, e),
        }
        out_file
    }
}


pub fn find_windows_sdk_tool_impl(_: &str) -> Option<PathBuf> {
    None
}
