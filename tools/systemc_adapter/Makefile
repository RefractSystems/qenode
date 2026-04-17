.PHONY: all clean

all: adapter

adapter:
	mkdir -p build
	cd build && cmake .. -DGIT_EXECUTABLE=$(shell which git) -DCMAKE_POLICY_VERSION_MINIMUM=3.5 && make

clean:
	rm -rf build
