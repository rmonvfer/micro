//! The Linux sandbox helper as a program of its own.

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
