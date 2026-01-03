use itertools::Itertools;
use miniz_oxide::inflate::decompress_to_vec_zlib;
use rayon::prelude::*;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write}; //引入DEFLATE库去解析IDAT
use std::os::unix::io::AsRawFd;
use terminal_size::{Height, Width, terminal_size};

struct PngInfo {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    compression: u8,
    filter_method: u8,
    interlace: u8,
    data: Vec<u8>,
}

fn parse_png(path: &str) -> io::Result<PngInfo> {
    let mut file = File::open(path)?;

    let mut signature = [0u8; 8];
    file.read_exact(&mut signature)?;
    if signature != [137, 80, 78, 71, 13, 10, 26, 10] {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "不是合法的 PNG 文件",
        ));
    }

    let mut chunk_len_buf = [0u8; 4];
    file.read_exact(&mut chunk_len_buf)?;
    let _length = u32::from_be_bytes(chunk_len_buf);

    // 再读 4 字节类型
    // 确认当前数据块的类型，是IHDR，IDAT还是IEND。
    let mut chunk_type = [0u8; 4];
    file.read_exact(&mut chunk_type)?;
    if &chunk_type != b"IHDR" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "缺少 IHDR 块"));
    }

    let mut width_buf = [0u8; 4];
    let mut height_buf = [0u8; 4];
    let mut other_buf = [0u8; 5];

    file.read_exact(&mut width_buf)?;
    file.read_exact(&mut height_buf)?;
    file.read_exact(&mut other_buf)?;

    file.seek(SeekFrom::Current(4))?;

    let mut idat_data: Vec<u8> = Vec::new();

    loop {
        let mut len_buf = [0u8; 4];
        file.read_exact(&mut len_buf)?;
        let length = u32::from_be_bytes(len_buf);

        let mut type_buf = [0u8; 4];
        file.read_exact(&mut type_buf)?;

        match &type_buf {
            b"IDAT" => {
                // println!("发现 IDAT 块，长度: {}", length);
                // 稍微高级一点的写法
                let mut reader = std::io::Read::by_ref(&mut file).take(length as u64);
                reader.read_to_end(&mut idat_data)?;
                file.seek(SeekFrom::Current(4))?;
            }
            b"IEND" => {
                // println!("发现 IEND 块，解析完毕。");

                file.seek(SeekFrom::Current(4))?;
                break;
            }
            _ => {
                let type_str = std::str::from_utf8(&type_buf).unwrap_or("????");
                println!("跳过块: {}, 长度: {}", type_str, length);
                // 跳过 数据内容 + 4字节CRC
                file.seek(SeekFrom::Current(length as i64 + 4))?;
            }
        }
    }

    let decompressed_data = decompress_to_vec_zlib(&idat_data)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("解压失败: {:?}", e)))?;
    Ok(PngInfo {
        width: u32::from_be_bytes(width_buf),
        height: u32::from_be_bytes(height_buf),
        bit_depth: other_buf[0],
        color_type: other_buf[1],
        compression: other_buf[2],
        filter_method: other_buf[3],
        interlace: other_buf[4],
        data: decompressed_data,
    })
}

fn reconstruct_pixels(info: &PngInfo) -> Vec<u8> {
    let width = info.width as usize;
    let height = info.height as usize;
    let bpp = 4; // Bytes Per Pixel, RGBA 是 4
    let line_size = width * bpp + 1; // 包含行首的 filter byte

    let mut recon = vec![0u8; width * height * bpp];
    let compressed_data = &info.data;

    for r in 0..height {
        let start = r * line_size;
        let filter_type = compressed_data[start];

        let scanline = &compressed_data[start + 1..start + line_size];

        let recon_start = r * width * bpp;

        for c in 0..(width * bpp) {
            let a = if c >= bpp {
                recon[recon_start + c - bpp]
            } else {
                0
            };
            let b = if r > 0 {
                recon[recon_start - (width * bpp) + c]
            } else {
                0
            };
            let c_val = if r > 0 && c >= bpp {
                recon[recon_start - (width * bpp) + c - bpp]
            } else {
                0
            };

            let filt = scanline[c];

            let recon_byte = match filter_type {
                0 => filt,
                1 => filt.wrapping_add(a),
                2 => filt.wrapping_add(b),
                3 => filt.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => filt.wrapping_add(paeth_predictor(a, b, c_val)),
                _ => panic!("未知的过滤类型"),
            };

            recon[recon_start + c] = recon_byte;
        }
    }
    recon
}

fn paeth_predictor(a: u8, b: u8, c: u8) -> u8 {
    let a = a as i16;
    let b = b as i16;
    let c = c as i16;

    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();

    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

fn print_as_sixel(width: u32, height: u32, pixels: &[u8]) {
    let bpp = 4;

    print!("\x1bPq#0;2;100;100;100");

    let bands: Vec<String> = (0..height)
        .step_by(6)
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|y_band| {
            let mut row_output = String::new();
            let mut sixel_chars = Vec::with_capacity(width as usize);

            for x in 0..width {
                let mut sixel_byte = 0u8;
                for bit in 0..6 {
                    let y = y_band + bit;
                    if y < height {
                        let idx = (y as usize * width as usize + x as usize) * bpp;
                        let brightness =
                            (pixels[idx] as u32 + pixels[idx + 1] as u32 + pixels[idx + 2] as u32)
                                / 3;
                        if brightness > 128 {
                            sixel_byte |= 1 << bit;
                        }
                    }
                }
                sixel_chars.push((sixel_byte + 63) as char);
            }

            row_output.push('#'); // 选择 0 号色
            row_output.push('0');

            // 使用 itertools 的 group_by 自动寻找连续相同的字符
            for (ch, group) in &sixel_chars.into_iter().chunk_by(|&c| c) {
                let count = group.count();
                if count > 3 {
                    row_output.push_str(&format!("!{}{}", count, ch));
                } else {
                    for _ in 0..count {
                        row_output.push(ch);
                    }
                }
            }
            row_output.push('-'); // 换行符
            row_output
        })
        .collect();

    // 4. 按顺序一次性输出
    for band in bands {
        print!("{}", band);
    }

    // 退出 Sixel 模式
    print!("\x1b\\");

    // io::stdout().flush().unwrap();
}

fn main() {
    match parse_png("imgs/girl.png") {
        Ok(info) => {
            // println!("PNG 宽度: {}", info.width);
            // println!("PNG 高度: {}", info.height);
            // println!("位深度: {}", info.bit_depth);
            // println!("颜色类型: {}", info.color_type); // 2 是 RGB, 6 是 RGBA
            // println!("压缩方法: {}", info.compression);
            // println!("过滤方法: {}", info.filter_method);
            // println!("隔行扫描: {}", info.interlace);
            // println!("解压后的原始数据长度: {} 字节", info.data.len());

            let final_pixels = reconstruct_pixels(&info);
            // println!("最终像素数组长度: {}", final_pixels.len());

            print_as_sixel(info.width, info.height, &final_pixels);

            io::stdout().flush().unwrap();
        }
        Err(e) => eprintln!("错误: {}", e),
    }
}
