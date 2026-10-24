// SPDX-License-Identifier: GPL-3.0-or-later

//! t2l command line interface.

mod cli;

fn main() {
    std::process::exit(cli::run());
}
