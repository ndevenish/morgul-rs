#pragma once

#include <memory>

#include "rust/cxx.h"
#include "sls/Receiver.h"
#include "sls_receiver/src/lib.rs.h"

struct StartHeader;
struct EndHeader;

class Receiver {
 public:
  Receiver(uint16_t port) : _receiver(port) {}

  rust::String getReceiverVersion() { return {_receiver.getReceiverVersion()}; }

  void registerCallBackStartAcquisition(rust::Fn<int(StartHeader)> callback);
  void registerCallBackEndAcquisition(rust::Fn<void(EndHeader)> callback);

 private:
  sls::Receiver _receiver;
  rust::Fn<int(StartHeader)> _start_callback;
  rust::Fn<void(EndHeader)> _end_callback;
  StartHeader _last_startheader;
};

auto make_receiver(uint16_t port) -> std::unique_ptr<Receiver>;
