#![allow(dead_code)]

#[cxx::bridge]
mod ffi {
    //     // struct startCallbackHeader {
    //     //     std::vector<uint32_t> udpPort;
    //     //     uint32_t dynamicRange;
    //     //     xy detectorShape;
    //     //     size_t imageSize;
    //     //     std::string filePath;
    //     //     std::string fileName;
    //     //     uint64_t fileIndex;
    //     //     bool quad;
    //     //     std::map<std::string, std::string> addJsonHeader;
    //     // };

    struct StartHeader {
        udp_port: Vec<u16>,
        dynamic_range: u32,
        detector_shape: [u32; 2],
        image_size: usize,
    }

    unsafe extern "C++" {
        include!("sls_receiver/include/sls_receiver_util.h");

        type Receiver;
        fn make_receiver(port: u16) -> UniquePtr<Receiver>;
        // fn getReceiverVersion(&self) -> String;
        // fn registerCallBackStartAcquisition(self: Pin<&mut Receiver>)
        // void registerCallBackStartAcquisition(rust::Fn<int(StartHeader)> callback);
    }

    // template <typename Signature>
    // class Fn;

    // template <typename Ret, typename... Args>
    // class Fn<Ret(Args...)> final {
    // public:
    //   Ret operator()(Args... args) const noexcept;
    //   Fn operator*() const noexcept;
    // };

    // unsafe extern "C++" {
    //     // include!("sls/Receiver.h");

    //     // type Receiver;

    //     // fn getReceiverVersion(self: Pin<&mut Receiver>) -> String;
    //     // void registerCallBackStartAcquisition(int (*func)(const startCallbackHeader,
    //     //                                                   void *),
    //     //                                       void *arg);
    //     // void registerCallBackAcquisitionFinished(
    //     //     void (*func)(const endCallbackHeader, void *), void *arg);
    //     // void registerCallBackRawDataReady(void (*func)(sls_receiver_header &,
    //     //                                                const dataCallbackHeader,
    //     //                                                char *, size_t &, void *),
    //     //                                   void *arg);
    // }
}

#[cfg(test)]
mod tests {
    use crate::ffi::*;

    #[test]
    fn test_create() {
        let r = make_receiver(30001);
        println!("Got receiver version: {}", r.getReceiverVersion());
    }
}
