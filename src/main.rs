use image::GenericImageView;
use image::imageops::FilterType;
use rayon::prelude::*;
use std::io::{self, Write};
use terminal_size::{Width, terminal_size};

fn main() {
    let img_path = "imgs/girl.png";
    let img = match image::open(img_path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("无法打开图片: {}", e);
            return;
        }
    };

    let (new_w, new_h) = get_scaled_dimensions(img.width(), img.height());
    let resized_img = img.resize(new_w, new_h, FilterType::Triangle);
    let (w, h) = resized_img.dimensions();
    let rgba = resized_img.to_rgba8();

    print_as_sixel_truecolor(w, h, &rgba);

    // 刷新缓冲区，确保终端能收到完整的 Sixel 数据流
    let _ = io::stdout().flush();
}

fn print_as_sixel_truecolor(width: u32, height: u32, pixels: &[u8]) {
    print!("\x1bPq");

    // 定义 256 色分布调色板: 8(R) * 8(G) * 4(B)
    // Sixel 的 RGB 分量通常是 0-100 (百分比)
    for i in 0..256u32 {
        let r = ((i >> 5) & 0x07) * 100 / 7;
        let g = ((i >> 2) & 0x07) * 100 / 7;
        let b = (i & 0x03) * 100 / 3;
        print!("#{};2;{};{};{}", i, r, g, b);
    }

    // 并行处理每个 6 像素高的“位带”
    let bands: Vec<String> = (0..height)
        .step_by(6)
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|y_band| {
            let mut band_output = String::new();
            // 准备 256 层颜色位图
            let mut color_layers = vec![vec![0u8; width as usize]; 256];

            for x in 0..width {
                for bit in 0..6 {
                    let y = y_band + bit;
                    if y < height {
                        let idx = (y as usize * width as usize + x as usize) * 4;
                        let r = pixels[idx] as usize;
                        let g = pixels[idx + 1] as usize;
                        let b = pixels[idx + 2] as usize;
                        let a = pixels[idx + 3];

                        if a > 128 {
                            // 将 24位 RGB 映射到 256 色索引
                            let r_idx = (r * 7 / 255) << 5;
                            let g_idx = (g * 7 / 255) << 2;
                            let b_idx = b * 3 / 255;
                            let color_idx = r_idx | g_idx | b_idx;

                            color_layers[color_idx][x as usize] |= 1 << bit;
                        }
                    }
                }
            }

            for (idx, layer) in color_layers.into_iter().enumerate() {
                if layer.iter().all(|&b| b == 0) {
                    continue;
                }

                band_output.push_str(&format!("#{}", idx));

                let mut x = 0;
                while x < width as usize {
                    let byte = layer[x];
                    let mut count = 1;
                    while x + count < width as usize && layer[x + count] == byte {
                        count += 1;
                    }
                    let sixel_char = (byte + 63) as char;
                    if count > 3 {
                        band_output.push_str(&format!("!{}{}", count, sixel_char));
                    } else {
                        for _ in 0..count {
                            band_output.push(sixel_char);
                        }
                    }
                    x += count;
                }
                band_output.push('$'); // 颜色层绘制完回车
            }
            band_output.push('-'); // 位带绘制完换行
            band_output
        })
        .collect();

    for band in bands {
        print!("{}", band);
    }
    print!("\x1b\\"); // 退出 Sixel 模式
}

fn get_char_pixel_size() -> Option<(u32, u32)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        // 使用标准输出 (fd 1) 获取窗口尺寸
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 {
            if ws.ws_xpixel > 0 && ws.ws_ypixel > 0 {
                let char_w = ws.ws_xpixel as u32 / ws.ws_col as u32;
                let char_h = ws.ws_ypixel as u32 / ws.ws_row as u32;
                return Some((char_w, char_h));
            }
        }
    }
    None
}

fn get_scaled_dimensions(img_w: u32, img_h: u32) -> (u32, u32) {
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

// 中值切割算法 (Median Cut),根据图片内容动态生成最优的 256 个颜色

// fn parse_png(path: &str) -> io::Result<PngInfo> {
//     let mut file = File::open(path)?;

//     let mut signature = [0u8; 8];
//     file.read_exact(&mut signature)?;
//     if signature != [137, 80, 78, 71, 13, 10, 26, 10] {
//         return Err(io::Error::new(
//             io::ErrorKind::InvalidData,
//             "不是合法的 PNG 文件",
//         ));
//     }

//     let mut chunk_len_buf = [0u8; 4];
//     file.read_exact(&mut chunk_len_buf)?;
//     let _length = u32::from_be_bytes(chunk_len_buf);

//     // 再读 4 字节类型
//     // 确认当前数据块的类型，是IHDR，IDAT还是IEND。
//     let mut chunk_type = [0u8; 4];
//     file.read_exact(&mut chunk_type)?;
//     if &chunk_type != b"IHDR" {
//         return Err(io::Error::new(io::ErrorKind::InvalidData, "缺少 IHDR 块"));
//     }

//     let mut width_buf = [0u8; 4];
//     let mut height_buf = [0u8; 4];
//     let mut other_buf = [0u8; 5];

//     file.read_exact(&mut width_buf)?;
//     file.read_exact(&mut height_buf)?;
//     file.read_exact(&mut other_buf)?;

//     file.seek(SeekFrom::Current(4))?;

//     let mut idat_data: Vec<u8> = Vec::new();

//     loop {
//         let mut len_buf = [0u8; 4];
//         file.read_exact(&mut len_buf)?;
//         let length = u32::from_be_bytes(len_buf);

//         let mut type_buf = [0u8; 4];
//         file.read_exact(&mut type_buf)?;

//         match &type_buf {
//             b"IDAT" => {
//                 // println!("发现 IDAT 块，长度: {}", length);
//                 // 稍微高级一点的写法
//                 let mut reader = std::io::Read::by_ref(&mut file).take(length as u64);
//                 reader.read_to_end(&mut idat_data)?;
//                 file.seek(SeekFrom::Current(4))?;
//             }
//             b"IEND" => {
//                 // println!("发现 IEND 块，解析完毕。");

//                 file.seek(SeekFrom::Current(4))?;
//                 break;
//             }
//             _ => {
//                 let type_str = std::str::from_utf8(&type_buf).unwrap_or("????");
//                 println!("跳过块: {}, 长度: {}", type_str, length);
//                 // 跳过 数据内容 + 4字节CRC
//                 file.seek(SeekFrom::Current(length as i64 + 4))?;
//             }
//         }
//     }

//     let decompressed_data = decompress_to_vec_zlib(&idat_data)
//         .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("解压失败: {:?}", e)))?;

//     Ok(PngInfo {
//         width: u32::from_be_bytes(width_buf),
//         height: u32::from_be_bytes(height_buf),
//         // bit_depth: other_buf[0],
//         // color_type: other_buf[1],
//         // compression: other_buf[2],
//         // filter_method: other_buf[3],
//         // interlace: other_buf[4],
//         data: decompressed_data,
//     })
// }
