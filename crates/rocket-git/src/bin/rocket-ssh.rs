fn main() {
    match rocket_git::ssh_helper_main(std::env::args_os().skip(1).collect()) {
        Ok(code) => std::process::exit(code),
        Err(error) => {
            // Error Display contains only static SSH guidance; never echo inputs.
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
