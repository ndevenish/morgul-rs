use std::path::Path;

use cxx_build::CFG;

fn main() {
    // CFG.exported_header_dirs.push(Path::new());
    cxx_build::bridge("src/lib.rs")
        .include("slsDetectorPackage/slsReceiverSoftware/include")
        .include("slsDetectorPackage/slsSupportLib/include")
        .file("src/sls_receiver_util.cc")
        .std("c++14")
        .compile("sls_receiver");

    println!("cargo:rustc-link-lib=SlsReceiver");
}
