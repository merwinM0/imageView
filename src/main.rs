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
    // bit_depth: u8,
    // color_type: u8,
    // compression: u8,
    // filter_method: u8,
    // interlace: u8,
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
        // bit_depth: other_buf[0],
        // color_type: other_buf[1],
        // compression: other_buf[2],
        // filter_method: other_buf[3],
        // interlace: other_buf[4],
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

fn get_char_pixel_size() -> Option<(u32, u32)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        // 0 代表标准输入 (stdin)
        if libc::ioctl(0, libc::TIOCGWINSZ, &mut ws) == 0 {
            if ws.ws_xpixel > 0 && ws.ws_ypixel > 0 {
                let char_w = ws.ws_xpixel as u32 / ws.ws_col as u32;
                let char_h = ws.ws_ypixel as u32 / ws.ws_row as u32;
                return Some((char_w, char_h));
            }
        }
    }
    None // 如果返回 0，说明终端不支持像素查询
}

fn get_scaled_dimensions(img_w: u32, img_h: u32) -> (u32, u32) {
    // 优先尝试获取真实像素，拿不到就默认按 8 像素算
    let (pixel_per_char, _) = get_char_pixel_size().unwrap_or((8, 16));

    if let Some((Width(tw), _)) = terminal_size() {
        let terminal_width_px = (tw as u32) * pixel_per_char;
        if img_w > terminal_width_px {
            let scale = terminal_width_px as f32 / img_w as f32;
            return (terminal_width_px, (img_h as f32 * scale) as u32);
        }
    }
    (img_w, img_h)
}

//最近邻缩放
fn scale_pixels(pixels: &[u8], old_w: u32, old_h: u32, new_w: u32, new_h: u32) -> Vec<u8> {
    let bpp = 4;
    (0..new_h)
        .into_par_iter()
        .flat_map(|y| {
            let mut row_pixels = Vec::with_capacity((new_w * bpp) as usize);
            let old_y = (y as f32 * (old_h as f32 / new_h as f32)) as u32;

            for x in 0..new_w {
                let old_x = (x as f32 * (old_w as f32 / new_w as f32)) as u32;
                let idx = ((old_y * old_w + old_x) * bpp) as usize;
                row_pixels.extend_from_slice(&pixels[idx..idx + bpp as usize]);
            }
            row_pixels
        })
        .collect()
}
fn print_as_sixel_color(width: u32, height: u32, pixels: &[u8]) {
    let bpp = 4;
    // 1. 打印 Sixel 引入头并定义一个简单的调色板 (8色)
    // 格式: #index;2;R;G;B (0-100)
    print!(
        "\x1bPq#0;2;0;0;0#1;2;100;0;0#2;2;0;100;0#3;2;100;100;0#4;2;0;0;100#5;2;100;0;100#6;2;0;100;100#7;2;100;100;100"
    );

    // 2. 并行处理每一个 6 像素高的“位带”
    let bands: Vec<String> = (0..height)
        .step_by(6)
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|y_band| {
            let mut band_output = String::new();

            // 为 8 种颜色分别建立位图缓存
            let mut color_layers = vec![vec![0u8; width as usize]; 8];

            for x in 0..width {
                for bit in 0..6 {
                    let y = y_band + bit;
                    if y < height {
                        let idx = (y as usize * width as usize + x as usize) * bpp;
                        let r = pixels[idx];
                        let g = pixels[idx + 1];
                        let b = pixels[idx + 2];
                        let a = pixels[idx + 3];

                        if a > 128 {
                            // 简单的颜色分类逻辑 (将 0-255 映射到 0 或 1)
                            let r_bit = if r > 128 { 1 } else { 0 };
                            let g_bit = if g > 128 { 2 } else { 0 };
                            let b_bit = if b > 128 { 4 } else { 0 };
                            let color_idx = r_bit | g_bit | b_bit; // 得到 0-7 的索引

                            color_layers[color_idx][x as usize] |= 1 << bit;
                        }
                    }
                }
            }

            // 3. 将各颜色层转化为 Sixel 字符串
            for (idx, layer) in color_layers.into_iter().enumerate() {
                // 如果这一层全是空的（没有这个颜色的像素），跳过以节省带宽
                if layer.iter().all(|&b| b == 0) {
                    continue;
                }

                band_output.push_str(&format!("#{}", idx)); // 切换颜色

                // 使用你原来的 itertools 分组优化
                for (ch_byte, group) in &layer.into_iter().chunk_by(|&b| b) {
                    let count = group.count();
                    let sixel_char = (ch_byte + 63) as char;
                    if count > 3 {
                        band_output.push_str(&format!("!{}{}", count, sixel_char));
                    } else {
                        for _ in 0..count {
                            band_output.push(sixel_char);
                        }
                    }
                }
                band_output.push('$'); // 每一层画完回到行首
            }
            band_output.push('-'); // 整个 6 像素带处理完，换行
            band_output
        })
        .collect();

    // 4. 按顺序输出
    for band in bands {
        print!("{}", band);
    }
    print!("\x1b\\"); // 退出 Sixel
}

fn main() {
    match parse_png("imgs/girl.png") {
        Ok(info) => {
            let original_pixels = reconstruct_pixels(&info);
            let (new_w, new_h) = get_scaled_dimensions(info.width, info.height);
            let final_pixels = if new_w != info.width || new_h != info.height {
                scale_pixels(&original_pixels, info.width, info.height, new_w, new_h)
            } else {
                original_pixels
            };
            print_as_sixel_color(new_w, new_h, &final_pixels);

            io::stdout().flush().unwrap();
        }
        Err(e) => eprintln!("错误: {}", e),
    }
}
