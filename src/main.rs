// SPDX-License-Identifier: GPL-3.0-or-later

//! t2l command line interface.

mod cli;
mod client;
mod daemon;
mod proto;

fn main() {
    std::process::exit(cli::run());
}
