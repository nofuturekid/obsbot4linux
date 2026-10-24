// SPDX-License-Identifier: GPL-3.0-or-later

//! obsbot4linux command line interface.

mod cli;
mod client;
mod daemon;
mod proto;

fn main() {
    std::process::exit(cli::run());
}
