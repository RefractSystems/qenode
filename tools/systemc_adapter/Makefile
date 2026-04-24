.PHONY: all clean adapter

# Source BUILD_DEPS if it exists to get pinned commits
ifneq (,$(wildcard ../../BUILD_DEPS))
    include ../../BUILD_DEPS
    export $(shell sed 's/=.*//' ../../BUILD_DEPS)
endif

# Fallbacks if BUILD_DEPS is not found
SYSTEMC_VERSION ?= 2.3.3
LIBSYSTEMCTLM_SOC_COMMIT ?= 7aba4f60e382581060c982976010674c33bdc442

all: adapter

adapter:
	mkdir -p build
	@if [ -f build/CMakeCache.txt ] && ! grep -q "CMAKE_HOME_DIRECTORY:INTERNAL=$(CURDIR)" build/CMakeCache.txt; then \
		echo "Wiping systemc_adapter build directory due to path change (e.g. host to container)"; \
		rm -rf build/*; \
	fi
	cd build && cmake .. \
		-DGIT_EXECUTABLE=$(shell which git) \
		-DCMAKE_POLICY_VERSION_MINIMUM=3.5 \
		-DSYSTEMC_VERSION=$(SYSTEMC_VERSION) \
		-DLIBSYSTEMCTLM_SOC_COMMIT=$(LIBSYSTEMCTLM_SOC_COMMIT)
	$(MAKE) -C build

clean:
	rm -rf build
