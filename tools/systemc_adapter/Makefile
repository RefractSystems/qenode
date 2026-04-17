.PHONY: all clean

all: adapter

adapter:
	mkdir -p build
	cd build && cmake .. -DGIT_EXECUTABLE=$(shell which git) && make

clean:
	rm -rf build
