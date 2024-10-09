use clap::Parser;

#[derive(Parser, Debug)]
pub struct CliArgs {
    pub base_fs: String,
}
