dbtest: dbtest.go firewood.h
	go build dbtest.go

firewood.h: src/lib.rs
	if [ ! -x "$$(which cbindgen)" ]; then cargo install --force cbindgen; fi
	@rm "$@" 2>/dev/null || true
	cbindgen -o "$@"

clean:
	rm firewood.h dbtest
