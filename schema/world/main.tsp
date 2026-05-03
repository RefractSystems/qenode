import "@typespec/json-schema";
import "./models.tsp";

using VirtMcu.World;
using TypeSpec.JsonSchema;

/**
 * The VirtMCU World Specification
 */
namespace VirtMcu.World;

@jsonSchema
model WorldSchema is World;
