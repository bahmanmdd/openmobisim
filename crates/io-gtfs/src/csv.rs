//! A streaming CSV reader for GTFS files: `csv-core`'s state machine over a
//! fixed input buffer, one record at a time, fields as trimmed `&str`.
//!
//! GTFS is RFC 4180 CSV in UTF-8, often with a byte-order mark and CRLF line
//! ends; both are handled. A record that is not valid UTF-8 reads as empty
//! fields, which the caller counts as a bad row.

use std::io::{self, Read};

use csv_core::{ReadRecordResult, Reader};

const INPUT_BUFFER: usize = 1 << 16;

/// Reads one CSV file, header first.
pub(crate) struct Csv<R: Read> {
    input: R,
    reader: Reader,
    buf: Vec<u8>,
    start: usize,
    end: usize,
    eof: bool,
    out: Vec<u8>,
    ends: Vec<usize>,
    fields: usize,
    header: Vec<String>,
}

impl<R: Read> Csv<R> {
    /// A reader over `input`, having read its header.
    pub(crate) fn new(input: R) -> io::Result<Self> {
        let mut csv = Self {
            input,
            reader: Reader::new(),
            buf: vec![0; INPUT_BUFFER],
            start: 0,
            end: 0,
            eof: false,
            out: vec![0; 1024],
            ends: vec![0; 32],
            fields: 0,
            header: Vec::new(),
        };
        if csv.next()? {
            csv.header = (0..csv.fields)
                .map(|i| {
                    let name = csv.field(i);
                    name.strip_prefix('\u{feff}').unwrap_or(name).trim().to_string()
                })
                .collect();
        }
        Ok(csv)
    }

    /// The column called `name`, if the header has it.
    pub(crate) fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h == name)
    }

    /// Read the next record; `false` at the end of the file. Blank lines are
    /// skipped.
    pub(crate) fn next(&mut self) -> io::Result<bool> {
        loop {
            if !self.read_record()? {
                return Ok(false);
            }
            if !(self.fields == 1 && self.ends[0] == 0) {
                return Ok(true);
            }
        }
    }

    fn read_record(&mut self) -> io::Result<bool> {
        let (mut nout, mut nend) = (0usize, 0usize);
        loop {
            if self.start == self.end && !self.eof {
                self.start = 0;
                self.end = self.input.read(&mut self.buf)?;
                if self.end == 0 {
                    self.eof = true;
                }
            }
            let (result, nin, o, e) = self.reader.read_record(
                &self.buf[self.start..self.end],
                &mut self.out[nout..],
                &mut self.ends[nend..],
            );
            // `csv-core` writes each call's bytes where the slice given starts,
            // and gives field ends as offsets from the start of the record.
            self.start += nin;
            nout += o;
            nend += e;
            match result {
                ReadRecordResult::InputEmpty => {}
                ReadRecordResult::OutputFull => {
                    let len = self.out.len();
                    self.out.resize(len * 2, 0);
                }
                ReadRecordResult::OutputEndsFull => {
                    let len = self.ends.len();
                    self.ends.resize(len * 2, 0);
                }
                ReadRecordResult::Record => {
                    self.fields = nend;
                    return Ok(true);
                }
                ReadRecordResult::End => return Ok(false),
            }
        }
    }

    /// Field `i` of the current record, trimmed; empty if the record has no
    /// such field or it is not UTF-8.
    pub(crate) fn field(&self, i: usize) -> &str {
        if i >= self.fields {
            return "";
        }
        let start = if i == 0 { 0 } else { self.ends[i - 1] };
        std::str::from_utf8(&self.out[start..self.ends[i]]).map_or("", str::trim)
    }

    /// Field `column` of the current record, or empty if the column is absent.
    pub(crate) fn get(&self, column: Option<usize>) -> &str {
        column.map_or("", |c| self.field(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader that hands out one byte at a time: every record boundary falls
    /// across a refill.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.0.is_empty() || buf.is_empty() {
                return Ok(0);
            }
            buf[0] = self.0[0];
            self.0 = &self.0[1..];
            Ok(1)
        }
    }

    fn rows(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
        let mut csv = Csv::new(Trickle(text.as_bytes())).unwrap();
        let header = csv.header.clone();
        let mut out = Vec::new();
        while csv.next().unwrap() {
            out.push((0..csv.fields).map(|i| csv.field(i).to_string()).collect());
        }
        (header, out)
    }

    #[test]
    fn quotes_bom_crlf_blank_lines_and_a_missing_final_newline() {
        let text = "\u{feff}stop_id, stop_name ,x\r\n1,\"Dam, Centrum\",  3 \r\n\r\n2,\"say \"\"hi\"\"\",4";
        let (header, rows) = rows(text);
        assert_eq!(header, ["stop_id", "stop_name", "x"]);
        assert_eq!(rows, [vec!["1", "Dam, Centrum", "3"], vec!["2", "say \"hi\"", "4"]]);
    }

    #[test]
    fn long_fields_and_many_columns_grow_the_buffers() {
        let long = "x".repeat(5000);
        let many: Vec<String> = (0..100).map(|i| i.to_string()).collect();
        let text = format!("a,b\n{long},1\n{}\n", many.join(","));
        let (_, rows) = rows(&text);
        assert_eq!(rows[0][0].len(), 5000);
        assert_eq!(rows[1].len(), 100);
        assert_eq!(rows[1][99], "99");
    }
}
