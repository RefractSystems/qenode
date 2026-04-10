#!/usr/bin/env python3
import os
import sys


def patch_file(filepath, marker, insertion, after=False):
    with open(filepath, "r") as f:
        content = f.read()
    if insertion in content:
        return
    idx = content.find(marker)
    if idx == -1:
        print(f"Error: Could not find marker '{marker}' in {filepath}")
        sys.exit(1)
    if after:
        idx += len(marker)
    new_content = content[:idx] + insertion + content[idx:]
    with open(filepath, "w") as f:
        f.write(new_content)

def main():
    qemu = sys.argv[1]

    char_json = os.path.join(qemu, "qapi", "char.json")
    char_c = os.path.join(qemu, "chardev", "char.c")

    # 1. Add 'zenoh' to ChardevBackendKind
    marker0 = "# @memory: synonym for @ringbuf (since 1.5)"
    insertion0 = "\n#\n# @zenoh: zenoh virtual clock backend (since 10.0)"
    patch_file(char_json, marker0, insertion0, after=True)

    marker1 = "'ringbuf',"
    insertion1 = "\n            'zenoh',"
    patch_file(char_json, marker1, insertion1, after=True)

    # 2. Add ChardevZenohOptions
    marker2 = "            { 'name': 'memory', 'features': [ 'deprecated' ] } ] }"
    insertion2 = """

##
# @ChardevZenohOptions:
#
# virtmcu: Zenoh virtual clock chardev backend
#
# @node: The zenoh node ID
# @router: The zenoh router address (optional)
#
# Since: 10.0
##
{ 'struct': 'ChardevZenohOptions',
  'base': 'ChardevCommon',
  'data': {
    'node': 'str',
    '*router': 'str' } }

##
# @ChardevZenohWrapper:
#
# @data: Configuration info for zenoh chardevs
#
# Since: 10.0
##
{ 'struct': 'ChardevZenohWrapper',
  'data': { 'data': 'ChardevZenohOptions' } }
"""
    patch_file(char_json, marker2, insertion2, after=True)

    # 3. Add 'zenoh' to ChardevBackend discriminator
    marker3 = "'ringbuf': 'ChardevRingbufWrapper',"
    insertion3 = "\n            'zenoh': 'ChardevZenohWrapper',"
    patch_file(char_json, marker3, insertion3, after=True)

    # 4. Patch qemu_chardev_opts in chardev/char.c
    marker4 = """        },{
            .name = "size","""
    insertion4 = """        },{
            .name = "node",
            .type = QEMU_OPT_STRING,
        },{
            .name = "router",
            .type = QEMU_OPT_STRING,"""
    patch_file(char_c, marker4, insertion4, after=False)

if __name__ == "__main__":
    main()
