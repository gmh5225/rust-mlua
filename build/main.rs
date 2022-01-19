#![allow(unreachable_code)]

use std::env;
use std::fs::File;
use std::io::{Error, ErrorKind, Result, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg_attr(
    all(
        feature = "vendored",
        any(
            feature = "lua54",
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit"
        )
    ),
    path = "find_vendored.rs"
)]
#[cfg_attr(
    all(
        not(feature = "vendored"),
        any(
            feature = "lua54",
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit"
        )
    ),
    path = "find_normal.rs"
)]
#[cfg_attr(
    not(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit"
    )),
    path = "find_dummy.rs"
)]
mod find;

trait CommandExt {
    fn execute(&mut self) -> Result<()>;
}

impl CommandExt for Command {
    /// Execute the command and return an error if it exited with a failure status.
    fn execute(&mut self) -> Result<()> {
        self.status()
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::Other, "non-zero exit code"))
                }
            })
            .map_err(|_| {
                Error::new(
                    ErrorKind::Other,
                    format!("The command {:?} did not run successfully.", self),
                )
            })
    }
}

// `include_path` is optional as Lua headers can be also found in compiler standard paths
fn build_glue(include_path: Option<impl AsRef<Path>>) {
    let build_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    let mut config = cc::Build::new();
    if let Some(include_path) = include_path {
        config.include(include_path.as_ref());
    }

    // Compile and run glue.c
    let glue = build_dir.join("glue");

    config
        .get_compiler()
        .to_command()
        .arg("src/ffi/glue/glue.c")
        .arg("-o")
        .arg(&glue)
        .execute()
        .unwrap();

    Command::new(glue)
        .arg(build_dir.join("glue.rs"))
        .execute()
        .unwrap();
}

// When cross-compiling, we cannot use `build_glue` as we cannot run the generated
// executable.  Instead, let's take a stab at synthesizing the likely values.
// If you're cross-compiling and using a non-vendored library then there is a chance
// that the values selected here may be incorrect, but we have no way to determine
// that here.
fn generate_glue() -> Result<()> {
    let build_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let mut glue = File::create(build_dir.join("glue.rs"))?;
    writeln!(
        glue,
        "/* This file was generated by build/main.rs; do not modify by hand */"
    )?;
    writeln!(glue, "use std::os::raw::*;")?;

    writeln!(glue, "/* luaconf.h */")?;
    let pointer_bit_width: usize = env::var("CARGO_CFG_TARGET_POINTER_WIDTH")
        .unwrap()
        .parse()
        .unwrap();
    writeln!(
        glue,
        "pub const LUA_EXTRASPACE: c_int = {} / 8;",
        pointer_bit_width
    )?;

    // This is generally hardcoded to this size
    writeln!(glue, "pub const LUA_IDSIZE: c_int = 60;")?;

    // Unless the target is restricted, the defaults are 64 bit
    writeln!(glue, "pub type LUA_NUMBER = c_double;")?;
    writeln!(glue, "pub type LUA_INTEGER = i64;")?;
    writeln!(glue, "pub type LUA_UNSIGNED = u64;")?;

    writeln!(glue, "/* lua.h */")?;
    let version = if cfg!(any(feature = "luajit", feature = "lua51")) {
        (5, 1, 0)
    } else if cfg!(feature = "lua52") {
        (5, 2, 0)
    } else if cfg!(feature = "lua53") {
        (5, 3, 0)
    } else if cfg!(feature = "lua54") {
        (5, 4, 0)
    } else {
        unreachable!();
    };
    writeln!(
        glue,
        "pub const LUA_VERSION_NUM: c_int = {};",
        (version.0 * 100) + version.1
    )?;

    #[cfg(any(feature = "lua54", feature = "lua53", feature = "lua52"))]
    writeln!(
        glue,
        "pub const LUA_REGISTRYINDEX: c_int = -{} - 1000;",
        if pointer_bit_width >= 32 {
            1_000_000
        } else {
            15_000
        }
    )?;
    #[cfg(any(feature = "lua51", feature = "luajit"))]
    writeln!(glue, "pub const LUA_REGISTRYINDEX: c_int = -10000;")?;

    // These two are only defined in lua 5.1
    writeln!(glue, "pub const LUA_ENVIRONINDEX: c_int = -10001;")?;
    writeln!(glue, "pub const LUA_GLOBALSINDEX: c_int = -10002;")?;

    writeln!(glue, "/* lauxlib.h */")?;
    // This is only defined in lua 5.3 and up, but we can always generate its value here,
    // even if we don't use it.
    // This matches the default definition in lauxlib.h
    writeln!(glue, "pub const LUAL_NUMSIZES: c_int = std::mem::size_of::<LUA_INTEGER>() as c_int * 16 + std::mem::size_of::<LUA_NUMBER>() as c_int;")?;

    writeln!(glue, "/* lualib.h */")?;
    write!(
        glue,
        r#"
#[cfg(feature = "luajit")]
pub const LUA_BITLIBNAME: &str = "bit";
#[cfg(not(feature = "luajit"))]
pub const LUA_BITLIBNAME: &str = "bit32";

pub const LUA_COLIBNAME: &str = "coroutine";
pub const LUA_DBLIBNAME: &str = "debug";
pub const LUA_IOLIBNAME: &str = "io";
pub const LUA_LOADLIBNAME: &str = "package";
pub const LUA_MATHLIBNAME: &str = "math";
pub const LUA_OSLIBNAME: &str = "os";
pub const LUA_STRLIBNAME: &str = "string";
pub const LUA_TABLIBNAME: &str = "table";
pub const LUA_UTF8LIBNAME: &str = "utf8";

pub const LUA_JITLIBNAME: &str = "jit";
pub const LUA_FFILIBNAME: &str = "ffi";
"#
    )?;

    Ok(())
}

fn main() {
    #[cfg(not(any(
        feature = "lua54",
        feature = "lua53",
        feature = "lua52",
        feature = "lua51",
        feature = "luajit"
    )))]
    compile_error!(
        "You must enable one of the features: lua54, lua53, lua52, lua51, luajit, luajit52"
    );

    #[cfg(all(
        feature = "lua54",
        any(
            feature = "lua53",
            feature = "lua52",
            feature = "lua51",
            feature = "luajit"
        )
    ))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52"
    );

    #[cfg(all(
        feature = "lua53",
        any(feature = "lua52", feature = "lua51", feature = "luajit")
    ))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52"
    );

    #[cfg(all(feature = "lua52", any(feature = "lua51", feature = "luajit")))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52"
    );

    #[cfg(all(feature = "lua51", feature = "luajit"))]
    compile_error!(
        "You can enable only one of the features: lua54, lua53, lua52, lua51, luajit, luajit52"
    );

    // We don't support "vendored module" mode on windows
    #[cfg(all(feature = "vendored", feature = "module", target_os = "windows"))]
    compile_error!(
        "Vendored (static) builds are not supported for modules on Windows.\n"
            + "Please, use `pkg-config` or custom mode to link to a Lua dll."
    );

    let include_dir = find::probe_lua();
    if env::var("TARGET").unwrap() != env::var("HOST").unwrap() {
        // The `probe_lua` call above is still needed here
        generate_glue().unwrap();
    } else {
        build_glue(include_dir);
        println!("cargo:rerun-if-changed=src/ffi/glue/glue.c");
    }

    println!("cargo:rerun-if-changed=build");
}
