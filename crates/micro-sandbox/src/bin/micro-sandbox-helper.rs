//! The Linux sandbox helper as a program of its own.
//!
//! A host that would rather not dispatch on [`micro_sandbox::HELPER_ARG`] at the top of
//! its own `main` points [`micro_sandbox::Sandbox::with_helper_program`] at this binary
//! instead. Either way the argument list is the same one, so both routes enforce the same
//! rules.

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next() {
        Some(marker) if marker == micro_sandbox::HELPER_ARG => {
            micro_sandbox::run_linux_helper(args)
        }
        _ => {
            eprintln!(
                "usage: micro-sandbox-helper {} --rules <json> -- <command>",
                micro_sandbox::HELPER_ARG
            );
            std::process::exit(micro_sandbox::HELPER_FAILURE_EXIT_CODE);
        }
    }
}
