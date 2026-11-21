install:
	cargo build --release && install -m 755 target/release/cascade ~/.local/bin && install -m 755 target/release/cascade-app ~/.local/bin

