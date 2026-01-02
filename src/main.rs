use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};

struct PngInfo {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    compression: u8,
    filter_method: u8,
    interlace: u8,
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

    // let mut idat_data: Vec<u8> = Vec::new();

    loop {
        let mut len_buf = [0u8; 4];
        file.read_exact(&mut len_buf)?;
        let length = u32::from_be_bytes(len_buf);

        let mut type_buf = [0u8; 4];
        file.read_exact(&mut type_buf)?;

        match &type_buf {
            b"IDAT" => {
                println!("发现 IDAT 块，长度: {}", length);
                file.seek(SeekFrom::Current(length as i64 + 4))?;
            }
            b"IEND" => {
                println!("发现 IEND 块，解析完毕。");
                // 读掉最后的 4 字节 CRC 校验，然后跳出循环
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

    Ok(PngInfo {
        width: u32::from_be_bytes(width_buf),
        height: u32::from_be_bytes(height_buf),
        bit_depth: other_buf[0],
        color_type: other_buf[1],
        compression: other_buf[2],
        filter_method: other_buf[3],
        interlace: other_buf[4],
    })
}

fn main() {
    match parse_png("imgs/girl.png") {
        Ok(info) => {
            println!("PNG 宽度: {}", info.width);
            println!("PNG 高度: {}", info.height);
            println!("位深度: {}", info.bit_depth);
            println!("颜色类型: {}", info.color_type); // 2 是 RGB, 6 是 RGBA
            println!("压缩方法: {}", info.compression);
            println!("过滤方法: {}", info.filter_method);
            println!("隔行扫描: {}", info.interlace);
        }
        Err(e) => eprintln!("错误: {}", e),
    }
}
