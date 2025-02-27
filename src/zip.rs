use crc::{CRC_32_ISO_HDLC, Crc, Digest};
use std::io::{self, Read, Seek, SeekFrom, Write};

const LOCAL_FILE_HEADER_SIGNATURE: &[u8] = b"\x50\x4b\x03\x04";
const CENTRAL_FILE_HEADER_SIGNATURE: &[u8] = b"\x50\x4b\x01\x02";
const EOF_CENTRAL_FILE_HEADER_SIGNATURE: &[u8] = b"\x50\x4b\x05\x06";
const VERSION_NEED_TO_EXTRACT_DEFAULT: &[u8] = b"\x00\x00";
const VERSION_MADE_BY: &[u8] = b"\x00\x3f"; // 6.3
const GENERAL_PURPOSE_BIT_FLAG: &[u8] = b"\x00\x00";
const COMPRESSION_METHOD_STORE: &[u8] = b"\x00\x00";
const LENGTH_ZERO: &[u8] = b"\x00\x00";
const INTERNAL_FILE_ATTRS: &[u8] = b"\x10\x00"; // text file
const EXTERNAL_FILE_ATTRS: &[u8] = b"\x00\x00\x00\x00";
const UNICODE_PATH_EXTRA_FIELD: &[u8] = b"\x75\x70";
const UNICODE_PATH_VERSION: &[u8] = b"\x01";

const CRC32: Crc<u32> = Crc::<u32>::new(&CRC_32_ISO_HDLC);

pub struct ZipWriter<W> {
    writer: W,
    files: Vec<FileEntry>,
    cursor: u64,
}

struct FileEntry {
    offset: u64,
    filename: Box<str>,
    size: u64,
    crc32: u32,
}

#[derive(Debug, PartialEq)]
enum FileHeader {
    Local,
    Central,
}

struct Utf8PathField<'a> {
    path: &'a str,
}

impl<'a> Utf8PathField<'a> {
    fn new(path: &'a str) -> Self {
        Utf8PathField { path }
    }

    fn into_bytes(self) -> Box<[u8]> {
        let mut buf = Vec::with_capacity(self.path.len() + 9);
        buf.write_all(UNICODE_PATH_EXTRA_FIELD).unwrap();
        buf.write_all(&((self.path.len() + 5) as u16).to_le_bytes())
            .unwrap();
        buf.write_all(UNICODE_PATH_VERSION).unwrap();

        let mut digest = CRC32.digest();
        digest.update(self.path.as_bytes());
        buf.write_all(&digest.finalize().to_le_bytes()).unwrap();

        buf.write_all(self.path.as_bytes()).unwrap();
        buf.into_boxed_slice()
    }
}

macro_rules! write_all {
    ($writer:expr, $count:ident, $buf:expr) => {
        $writer.write_all($buf)?;
        $count += $buf.len();
    };
}

impl FileHeader {
    fn signature(&self) -> &'static [u8] {
        match self {
            FileHeader::Local => LOCAL_FILE_HEADER_SIGNATURE,
            FileHeader::Central => CENTRAL_FILE_HEADER_SIGNATURE,
        }
    }
}

impl FileEntry {
    fn new(offset: u64, filename: Box<str>, size: u64, crc32: u32) -> Self {
        FileEntry {
            offset,
            filename,
            size,
            crc32,
        }
    }

    fn write_header<W>(&self, w: &mut W, header: FileHeader) -> io::Result<usize>
    where
        W: Write,
    {
        let mut n = 0;
        write_all!(w, n, header.signature());
        if header == FileHeader::Central {
            write_all!(w, n, VERSION_MADE_BY);
        }
        write_all!(w, n, VERSION_NEED_TO_EXTRACT_DEFAULT);
        write_all!(w, n, GENERAL_PURPOSE_BIT_FLAG);
        write_all!(w, n, COMPRESSION_METHOD_STORE);
        write_all!(w, n, b"\x00\x00\x00\x00"); // time & date
        write_all!(w, n, &self.crc32.to_le_bytes());
        let size_bytes = (self.size as u32).to_le_bytes();
        write_all!(w, n, &size_bytes);
        write_all!(w, n, &size_bytes);
        write_all!(w, n, &(self.filename.len() as u16).to_le_bytes());
        let extra = Utf8PathField::new(&self.filename).into_bytes();
        write_all!(w, n, &(extra.len() as u16).to_le_bytes());
        if header == FileHeader::Central {
            write_all!(w, n, LENGTH_ZERO); // file comment
            write_all!(w, n, LENGTH_ZERO); // disk number
            write_all!(w, n, INTERNAL_FILE_ATTRS);
            write_all!(w, n, EXTERNAL_FILE_ATTRS);
            write_all!(w, n, &(self.offset as u32).to_le_bytes());
        }
        write_all!(w, n, self.filename.as_bytes());
        write_all!(w, n, &extra);
        Ok(n)
    }
}

impl<W> ZipWriter<W>
where
    W: Write + Seek,
{
    pub fn new(writer: W) -> Self {
        ZipWriter {
            writer,
            files: Vec::new(),
            cursor: 0,
        }
    }

    pub fn write_file<R>(&mut self, filename: &str, content: R) -> io::Result<()>
    where
        R: Read,
    {
        // write local header
        let filename = filename.to_owned().into_boxed_str();
        let mut file = FileEntry::new(self.cursor, filename, 0, 0);
        self.cursor += file.write_header(&mut self.writer, FileHeader::Local)? as u64;

        // write file content
        let mut content = Crc32Reader::new(content);
        file.size = io::copy(&mut content, &mut self.writer)?;
        file.crc32 = content.sum32();
        self.cursor += file.size;

        // update header
        self.writer.seek(SeekFrom::Start(file.offset))?;
        file.write_header(&mut self.writer, FileHeader::Local)?;
        self.writer.seek(SeekFrom::Start(self.cursor))?;

        self.files.push(file);
        Ok(())
    }

    pub fn close(self) -> io::Result<()> {
        let ZipWriter {
            mut writer,
            files,
            cursor,
        } = self;

        let entries_len = (files.len().to_le() as u16).to_le_bytes();
        let mut len = 0;
        for file in files {
            len += file.write_header(&mut writer, FileHeader::Central)?;
        }

        writer.write_all(EOF_CENTRAL_FILE_HEADER_SIGNATURE)?;
        writer.write_all(LENGTH_ZERO)?; // number of this disk
        writer.write_all(&1u16.to_le_bytes())?; // disk w/ central dir
        writer.write_all(&entries_len)?; // in the central dir on this disk
        writer.write_all(&entries_len)?; // total in the central dir
        writer.write_all(&(len as u32).to_le_bytes())?;
        writer.write_all(&(cursor as u32).to_le_bytes())?;
        writer.write_all(LENGTH_ZERO)?; // zip file comment
        Ok(())
    }
}

struct Crc32Reader<R> {
    internal: R,
    digest: Digest<'static, u32>,
}

impl<R: Read> Crc32Reader<R> {
    fn new(internal: R) -> Self {
        Crc32Reader {
            internal,
            digest: CRC32.digest(),
        }
    }

    fn sum32(self) -> u32 {
        self.digest.finalize()
    }
}

impl<R: Read> Read for Crc32Reader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let len = self.internal.read(buf)?;
        self.digest.update(&buf[..len]);
        Ok(len)
    }
}
