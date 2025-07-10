#include "sls_receiver/include/sls_receiver_util.h"

#include <memory>

#include "sls/Receiver.h"
#include "sls_receiver/src/lib.rs.h"

int start_callback_trampoline(slsDetectorDefs::startCallbackHeader header,
                              void *arg) {
  Receiver &rec = *static_cast<Receiver *>(arg);
}

void end_callback_trampoline(slsDetectorDefs::endCallbackHeader header,
                             void *arg) {
  Receiver &rec = *static_cast<Receiver *>(arg);
}

void data_callback_trampoline(
    slsDetectorDefs::sls_receiver_header &rec_header,
    const slsDetectorDefs::dataCallbackHeader det_header, char *data,
    size_t &data_size, void *arg) {
  Receiver &rec = *static_cast<Receiver *>(arg);
}

auto Receiver::registerCallBackStartAcquisition(
    rust::Fn<int(StartHeader)> callback) -> void {
  _start_callback = callback;
  _receiver.registerCallBackStartAcquisition(start_callback_trampoline, this);
}

auto Receiver::registerCallBackEndAcquisition(
    rust::Fn<void(EndHeader)> callback) -> void {
  _end_callback = callback;
  _receiver.registerCallBackAcquisitionFinished(end_callback_trampoline, this);
}

auto make_receiver(uint16_t port) -> std::unique_ptr<Receiver> {
  return std::make_unique<Receiver>(port);
}
