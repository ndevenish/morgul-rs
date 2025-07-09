#pragma once

#include <memory>

#include "sls/Receiver.h"

namespace sls {
auto make_receiver(uint16_t port) -> std::unique_ptr<sls::Receiver>;
}