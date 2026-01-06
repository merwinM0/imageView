use image::imageops::FilterType;
use image::{GenericImageView, Rgba};
use rayon::prelude::*;
use std::io::{self, Write};
use terminal_size::{Width, terminal_size};

fn main() {
    let img_path = "imgs/girl_2.png";
    let img = match image::open(img_path) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("无法打开图片: {}", e);
            return;
        }
    };

    // 1. 终端自适应缩放 (使用高质量 Lanczos3 滤镜)
    let (new_w, new_h) = get_scaled_dimensions(img.width(), img.height());
    let resized_img = img.resize(new_w, new_h, FilterType::Lanczos3);

    // 2. 渲染带抖动效果的全彩 Sixel
    print_as_sixel_advanced(resized_img);

    let _ = io::stdout().flush();
}

fn print_as_sixel_advanced(img: image::DynamicImage) {
    let (width, height) = img.dimensions();
    print!("\x1bPq"); // 进入 Sixel

    // 1. 定义 256 色固定调色板 (8R * 8G * 4B)
    for i in 0..256u32 {
        let r = ((i >> 5) & 0x07) * 100 / 7;
        let g = ((i >> 2) & 0x07) * 100 / 7;
        let b = (i & 0x03) * 100 / 3;
        print!("#{};2;{};{};{}", i, r, g, b);
    }

    // 2. 预处理：执行 Floyd-Steinberg 抖动
    // 注意：抖动是序列化的过程，无法直接在位带内完全并发，我们对整图进行处理
    let mut pixels = img.to_rgba8();
    let mut error_buffer = vec![vec![[0f32; 3]; width as usize + 2]; height as usize + 1];

    for y in 0..height {
        for x in 0..width {
            let px = pixels.get_pixel(x, y);
            let r = px[0] as f32 + error_buffer[y as usize][x as usize + 1][0];
            let g = px[1] as f32 + error_buffer[y as usize][x as usize + 1][1];
            let b = px[2] as f32 + error_buffer[y as usize][x as usize + 1][2];

            // 寻找调色板中最接近的颜色
            let r_idx = (r.clamp(0.0, 255.0) * 7.0 / 255.0).round() as usize;
            let g_idx = (g.clamp(0.0, 255.0) * 7.0 / 255.0).round() as usize;
            let b_idx = (b.clamp(0.0, 255.0) * 3.0 / 255.0).round() as usize;

            let best_r = (r_idx * 255 / 7) as f32;
            let best_g = (g_idx * 255 / 7) as f32;
            let best_b = (b_idx * 255 / 3) as f32;

            // 计算误差
            let err = [r - best_r, g - best_g, b - best_b];

            // 分发误差 (Floyd-Steinberg 矩阵)
            distribute_error(&mut error_buffer, x as usize + 1, y as usize, err);

            // 更新像素为量化后的颜色
            pixels.put_pixel(
                x,
                y,
                Rgba([best_r as u8, best_g as u8, best_b as u8, px[3]]),
            );
        }
    }

    // 3. 并行生成 Sixel 数据流 (保持高效输出)
    let bands: Vec<String> = (0..height)
        .step_by(6)
        .collect::<Vec<_>>()
        .into_par_iter()
        .map(|y_band| {
            let mut band_output = String::new();
            let mut color_layers = vec![vec![0u8; width as usize]; 256];

            for x in 0..width {
                for bit in 0..6 {
                    let y = y_band + bit;
                    if y < height {
                        let px = pixels.get_pixel(x, y);
                        if px[3] > 128 {
                            let r_idx = (px[0] as usize * 7 / 255) << 5;
                            let g_idx = (px[1] as usize * 7 / 255) << 2;
                            let b_idx = px[2] as usize * 3 / 255;
                            color_layers[r_idx | g_idx | b_idx][x as usize] |= 1 << bit;
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
                band_output.push('$');
            }
            band_output.push('-');
            band_output
        })
        .collect();

    for band in bands {
        print!("{}", band);
    }
    print!("\x1b\\");
}

fn distribute_error(buffer: &mut Vec<Vec<[f32; 3]>>, x: usize, y: usize, err: [f32; 3]) {
    let height = buffer.len() - 1;
    let width = buffer[0].len() - 2;

    // 右方: 7/16
    if x < width {
        buffer[y][x + 1] = add_err(buffer[y][x + 1], err, 7.0 / 16.0);
    }
    // 下方行
    if y + 1 < height {
        // 左下: 3/16
        if x > 1 {
            buffer[y + 1][x - 1] = add_err(buffer[y + 1][x - 1], err, 3.0 / 16.0);
        }
        // 正下: 5/16
        buffer[y + 1][x] = add_err(buffer[y + 1][x], err, 5.0 / 16.0);
        // 右下: 1/16
        if x < width {
            buffer[y + 1][x + 1] = add_err(buffer[y + 1][x + 1], err, 1.0 / 16.0);
        }
    }
}

fn add_err(a: [f32; 3], b: [f32; 3], weight: f32) -> [f32; 3] {
    [
        a[0] + b[0] * weight,
        a[1] + b[1] * weight,
        a[2] + b[2] * weight,
    ]
}

fn get_char_pixel_size() -> Option<(u32, u32)> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) == 0 {
            if ws.ws_xpixel > 0 && ws.ws_ypixel > 0 {
                return Some((
                    ws.ws_xpixel as u32 / ws.ws_col as u32,
                    ws.ws_ypixel as u32 / ws.ws_row as u32,
                ));
            }
        }
    }
    None
}

fn get_scaled_dimensions(img_w: u32, img_h: u32) -> (u32, u32) {
    let (px_w, _) = get_char_pixel_size().unwrap_or((8, 16));
    if let Some((Width(tw), _)) = terminal_size() {
        let target_w = tw as u32 * px_w;
        if img_w > target_w {
            return (
                target_w,
                (img_h as f32 * (target_w as f32 / img_w as f32)) as u32,
            );
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
