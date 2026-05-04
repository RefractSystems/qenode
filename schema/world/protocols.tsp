import "@typespec/json-schema";

using TypeSpec.JsonSchema;

namespace VirtMcu.World;

/**
 * Supported inter-node communication protocols.
 */
@extension("pattern", "^(Ethernet|Uart|CanFd|Spi|FlexRay|Lin|Rf802154|RfHci|eth|uart|canfd|spi|flexray|lin|rf802154|rfhci|ethernet)$")
scalar Protocol extends string;
