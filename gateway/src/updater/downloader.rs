//! Update download functionality

use super::{UpdateError, VersionInfo};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

/// Downloads updates from a remote server
pub struct UpdateDownloader {
    download_base_url: String,
    temp_dir: PathBuf,
    client: reqwest::Client,
}

impl UpdateDownloader {
    /// Create a new UpdateDownloader
    pub fn new(download_base_url: String, temp_dir: PathBuf) -> Self {
        Self {
            download_base_url,
            temp_dir,
            client: reqwest::Client::new(),
        }
    }

    /// Download an update and return the path to the downloaded file
    pub async fn download(&self, version_info: &VersionInfo) -> Result<PathBuf, UpdateError> {
        // Create temp directory if it doesn't exist
        tokio::fs::create_dir_all(&self.temp_dir).await?;

        // Determine download URL
        let download_url = if version_info.download_url.starts_with("http") {
            version_info.download_url.clone()
        } else {
            format!("{}/{}", self.download_base_url, version_info.download_url)
        };

        tracing::debug!("Downloading update from: {}", download_url);

        // Download the file
        let response = self.client
            .get(&download_url)
            .header("User-Agent", format!("gateway/{}", env!("CARGO_PKG_VERSION")))
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UpdateError::Download(
                format!("Server returned status: {}", response.status())
            ));
        }

        // Determine filename
        let filename = self.extract_filename(&download_url, &version_info.version);
        let download_path = self.temp_dir.join(&filename);

        // Write to file
        let bytes = response.bytes().await?;

        // Verify checksum if provided
        if let Some(ref expected_checksum) = version_info.checksum {
            let actual_checksum = self.calculate_sha256(&bytes);
            if &actual_checksum != expected_checksum {
                return Err(UpdateError::Download(
                    format!("Checksum mismatch: expected {}, got {}", expected_checksum, actual_checksum)
                ));
            }
            tracing::debug!("Checksum verified: {}", actual_checksum);
        }

        let mut file = tokio::fs::File::create(&download_path).await?;
        file.write_all(&bytes).await?;
        file.flush().await?;

        tracing::info!("Update downloaded to {:?}", download_path);

        Ok(download_path)
    }

    /// Extract filename from URL or generate one based on version
    fn extract_filename(&self, url: &str, version: &str) -> String {
        url.rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                #[cfg(windows)]
                {
                    format!("gateway-{}.exe", version)
                }
                #[cfg(not(windows))]
                {
                    format!("gateway-{}", version)
                }
            })
    }

    /// Calculate SHA256 checksum of data
    fn calculate_sha256(&self, data: &[u8]) -> String {
        use std::fmt::Write;

        // Simple SHA256 implementation would go here
        // For now, we use a placeholder
        // In production, use the sha2 crate
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();

        let mut hex = String::with_capacity(64);
        for byte in result {
            write!(hex, "{:02x}", byte).unwrap();
        }
        hex
    }
}

// Simple SHA256 implementation (in production, use the sha2 crate)
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    total_len: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a,
                0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::new(),
            total_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        self.total_len += data.len() as u64;
    }

    fn finalize(mut self) -> [u8; 32] {
        // Padding
        let bit_len = self.total_len * 8;
        self.buffer.push(0x80);
        while (self.buffer.len() % 64) != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());

        // Process all blocks - collect chunks first to avoid borrow conflict
        let chunks: Vec<[u8; 64]> = self.buffer
            .chunks(64)
            .map(|c| c.try_into().unwrap())
            .collect();

        for chunk in chunks {
            self.process_block(&chunk);
        }

        // Output
        let mut result = [0u8; 32];
        for (i, &val) in self.state.iter().enumerate() {
            result[i * 4..(i + 1) * 4].copy_from_slice(&val.to_be_bytes());
        }
        result
    }

    fn process_block(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
            0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
            0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
            0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
            0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
            0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
            0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
        ];

        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes(block[i * 4..(i + 1) * 4].try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }

        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }
}
