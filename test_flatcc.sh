#!/bin/bash
set -e
git clone https://github.com/dvidelabs/flatcc.git /tmp/flatcc
cd /tmp/flatcc
./scripts/build.sh
ls -la bin/ lib/
