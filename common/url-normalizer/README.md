# URL Normalization Library

A Rust library for normalizing URLs by removing fragments, sorting query parameters, normalizing paths, removing default ports, and applying percent-encoding.

## Features

- **URL Parsing**: Robust URL parsing with error handling
- **Fragment Removal**: Automatically removes URL fragments
- **Query Parameter Sorting**: Sorts query parameters alphabetically
- **Path Normalization**: Removes consecutive slashes from paths
- **Default Port Removal**: Removes default ports (80 for HTTP, 443 for HTTPS, 21 for FTP)
- **Percent Encoding**: Applies proper percent-encoding to paths and query parameters
- **CLI Tool**: Command-line interface for URL normalization

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
url_normalizer = "0.1.0"
```

## Usage

### Basic Usage

```rust
use url_normalizer::normalize_url;

fn main() {
    let url = "https://example.com//path//to//file?b=2&a=1#fragment";
    
    match normalize_url(url) {
        Ok(normalized) => println!("Normalized: {}", normalized),
        Err(e) => eprintln!("Error: {}", e),
    }
}
```



## CLI Usage

The library also provides a command-line interface:

```bash
# Basic usage
cargo run -- "https://example.com//path//to//file?b=2&a=1#fragment"

# Help
cargo run -- --help
```

## Examples

### URL Normalization Examples

```rust
use url_normalizer::normalize_url;

// Remove fragments
let url = "https://example.com/path#fragment";
let normalized = normalize_url(url).unwrap();
assert_eq!(normalized, "https://example.com/path");

// Sort query parameters
let url = "https://example.com/path?b=2&a=1";
let normalized = normalize_url(url).unwrap();
assert_eq!(normalized, "https://example.com/path?a=1&b=2");

// Remove consecutive slashes
let url = "https://example.com//path//to//file";
let normalized = normalize_url(url).unwrap();
assert_eq!(normalized, "https://example.com/path/to/file");

// Remove default ports
let url = "https://example.com:443/path";
let normalized = normalize_url(url).unwrap();
assert_eq!(normalized, "https://example.com/path");

// Percent-encode special characters
let url = "https://example.com/path with spaces";
let normalized = normalize_url(url).unwrap();
assert_eq!(normalized, "https://example.com/path%20with%20spaces");
```

## API Reference

### Functions

- `normalize_url(url: impl AsRef<str>) -> Result<String, url::ParseError>`: Normalize a complete URL
  - Removes fragments
  - Sorts query parameters alphabetically
  - Normalizes paths (removes consecutive slashes)
  - Removes default ports (80 for HTTP, 443 for HTTPS, 21 for FTP)
  - Applies percent-encoding to paths and query parameters

## License

Licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for the full text.
