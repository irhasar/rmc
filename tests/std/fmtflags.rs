// Format-placeholder options: fill, alignment, width, precision, sign,
// alternate form and the integer bases. Every line is a shape the MIR path has
// to render exactly as rustc does.

fn main() {
    // Fill and alignment on integers.
    println!("[{:>5}]", 42);
    println!("[{:<5}]", 42);
    println!("[{:^5}]", 42);
    println!("[{:5}]", 42);
    println!("[{:*>6}]", 42);
    println!("[{:*<6}]", 42);
    println!("[{:*^6}]", 42);
    println!("[{:0>6}]", 42);
    println!("[{:->5}]", 42);

    // Zero padding keeps the sign and the base prefix leftmost.
    println!("[{:05}]", 42);
    println!("[{:05}]", -42);
    println!("[{:+05}]", 42);
    println!("[{:08x}]", 255);
    println!("[{:#08x}]", 255);
    println!("[{:#010b}]", 5);

    // Sign flags.
    println!("[{:+}]", 42);
    println!("[{:+}]", -42);
    println!("[{: }]", 42);
    println!("[{: }]", -42);

    // Alternate forms.
    println!("[{:#x}]", 255);
    println!("[{:#X}]", 255);
    println!("[{:#b}]", 5);
    println!("[{:#o}]", 8);
    println!("[{:#x}]", 0);
    println!("[{:+#x}]", 255);

    // Bases, with and without width.
    println!("[{:x}]", 255);
    println!("[{:X}]", 255);
    println!("[{:b}]", 5);
    println!("[{:o}]", 8);
    println!("[{:>8x}]", 255);

    // A narrower integer prints at its own width.
    println!("[{:x}]", -2i16);
    println!("[{:x}]", 255u8);

    // Strings default to left alignment, other values to right.
    println!("[{:5}]", "ab");
    println!("[{:<5}]", "ab");
    println!("[{:>5}]", "ab");
    println!("[{:^5}]", "ab");
    println!("[{:*^6}]", "ab");
    println!("[{:>8}]", "hello");

    // Booleans and chars read as text.
    println!("[{:5}]", true);
    println!("[{:>6}]", false);
    println!("[{:5}]", 'A');
    println!("[{:^5}]", 'A');

    // Mixed literal pieces and several placeholders in one template.
    println!("{} and {:>5} and {:#x}", 1, 2, 255);
    println!("{0} {1} {0}", 7, 8);
    println!("{name} = {value:>6}", name = "n", value = 42);
}
