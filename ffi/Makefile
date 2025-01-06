../target/release/libfirewood.so: src/lib.rs firewood.h
	cargo build --release

firewood.h: src/lib.rs
	if [ ! -x "$$(which cbindgen)" ]; then cargo install --force cbindgen; fi
	@rm "$@" 2>/dev/null || true
	cbindgen -o "$@"

clean:
	rm firewood.h 
