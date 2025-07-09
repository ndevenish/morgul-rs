#include "sls_receiver/include/sls_receiver_util.h"

namespace sls {
auto make_receiver(uint16_t port) -> std::unique_ptr<sls::Receiver> {
  return std::make_unique<sls::Receiver>(port);
}
}  // namespace sls