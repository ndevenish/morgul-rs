#[cxx::bridge(namespace = "sls")]
mod ffi {
    unsafe extern "C++" {
        include!("sls/Receiver.h");
        include!("sls_receiver/include/sls_receiver_util.h");

        type Receiver;

        fn make_receiver(port: u16) -> UniquePtr<Receiver>;

    }
}

#[cfg(test)]
mod tests {
    use crate::ffi::*;

    #[test]
    fn test_create() {
        let r = make_receiver(30001);
    }
}
