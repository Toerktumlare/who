use nom::number::complete::be_u16;
use nom::number::complete::be_u32;
use nom::sequence::tuple;
use nom::Finish;
use std::fmt::Display;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::time::Duration;

use super::parse_utils::parse_ipv4;
use super::parse_utils::parse_ipv6;
use super::parse_utils::parse_names;
use super::parse_utils::parse_qclass;
use super::parse_utils::parse_qtype;
use super::parse_utils::parse_rdlength;
use super::parse_utils::parse_ttl;
use super::parse_utils::take_token;
use super::parse_utils::VResult;
use super::Buffer;
use super::{DeSerialize, QClass, QType};

#[allow(clippy::upper_case_acronyms)]
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum RData {
    A(Ipv4Addr),
    CNAME(String),
    TXT(String),
    AAAA(Ipv6Addr),
    NS(String),
    MX {
        preference: u16,
        exchange: String,
    },
    SOA {
        mname: String,
        rname: String,
        serial: u32,
        refresh: u32,
        retry: u32,
        expire: u32,
        minimum: u32,
    },
}

impl Display for RData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RData::A(value) => write!(f, "{value}"),
            RData::CNAME(value) => write!(f, "{value}"),
            RData::TXT(value) => write!(f, "{value}"),
            RData::AAAA(value) => write!(f, "{value}"),
            RData::NS(value) => write!(f, "{value}"),
            RData::MX {
                preference,
                exchange,
            } => write!(f, "{preference} {exchange}"),
            RData::SOA {
                mname,
                rname,
                serial,
                refresh,
                retry,
                expire,
                minimum,
            } => write!(
                f,
                "{mname}, {rname}, {serial}, {refresh}, {retry}, {expire}, {minimum}"
            ),
        }
    }
}

// Resource record format
//
// The answer, authority, and additional sections all share the same
// format: a variable number of resource records, where the number of
// records is specified in the corresponding count field in the header.
// Each resource record has the following format:
//                                     1  1  1  1  1  1
//       0  1  2  3  4  5  6  7  8  9  0  1  2  3  4  5
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+
//     |                                               |
//     /                                               /
//     /                      NAME                     /
//     |                                               |
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+
//     |                      TYPE                     |
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+
//     |                     CLASS                     |
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+
//     |                      TTL                      |
//     |                                               |
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+
//     |                   RDLENGTH                    |
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--|
//     /                     RDATA                     /
//     /                                               /
//     +--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+--+
//
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    // a domain name to which this resource record pertains.
    pub name: String,

    // two octets containing one of the RR type codes.
    // This field specifies the meaning of the data in the RDATA field.
    pub qtype: QType,

    // two octets which specify the class of the data in the
    // RDATA field.
    pub qclass: QClass,

    // a 32 bit unsigned integer that specifies the time
    // interval (in seconds) that the resource record may be
    // cached before it should be discarded.  Zero values are
    // interpreted to mean that the RR can only be used for the
    // transaction in progress, and should not be cached.
    pub ttl: Duration,

    // an unsigned 16 bit integer that specifies the length in
    // octets of the RDATA field.
    pub rd_length: u16,

    // a variable length string of octets that describes the
    // resource.  The format of this information varies
    // according to the TYPE and CLASS of the resource record.
    // For example, the if the TYPE is A and the CLASS is IN,
    // the RDATA field is a 4 octet ARPA Internet address.
    pub rdata: RData,
}

impl Record {
    pub fn new(
        name: String,
        qtype: QType,
        qclass: QClass,
        ttl: Duration,
        rd_length: u16,
        rdata: RData,
    ) -> Self {
        Self {
            name,
            qtype,
            qclass,
            ttl,
            rd_length,
            rdata,
        }
    }
}

fn parse_record<'a>(buffer: &'a [u8], source: &'a [u8]) -> VResult<&'a [u8], Record> {
    let mut t = Vec::new();
    let (buffer, name) = parse_names(buffer, source, &mut t)?;

    let (buffer, (qtype, qclass, ttl, rd_length)) =
        tuple((parse_qtype, parse_qclass, parse_ttl, parse_rdlength))(buffer)?;

    let (buffer, rdata) = match qtype {
        QType::A => {
            let (buffer, address) = parse_ipv4(buffer)?;
            (buffer, RData::A(address))
        }
        QType::CNAME => {
            let mut t = Vec::new();
            let (buffer, name) = parse_names(buffer, source, &mut t)?;
            (buffer, RData::CNAME(name))
        }
        QType::TXT => {
            let (buffer, txt) = take_token(buffer, rd_length.into())?;
            (buffer, RData::TXT(txt.to_owned()))
        }
        QType::AAAA => {
            let (buffer, address) = parse_ipv6(buffer)?;
            (buffer, RData::AAAA(address))
        }
        QType::NS => {
            let (buffer, name) = parse_names(buffer, source, &mut t)?;
            (buffer, RData::NS(name))
        }
        QType::MX => {
            let (buffer, preference) = be_u16(buffer)?;
            let (buffer, exchange) = parse_names(buffer, source, &mut t)?;
            (
                buffer,
                RData::MX {
                    preference,
                    exchange,
                },
            )
        }
        QType::SOA => {
            let (buffer, mname) = parse_names(buffer, source, &mut t)?;
            let (buffer, rname) = parse_names(buffer, source, &mut t)?;
            let (buffer, (serial, refresh, retry, expire, minimum)) =
                tuple((be_u32, be_u32, be_u32, be_u32, be_u32))(buffer)?;
            (
                buffer,
                RData::SOA {
                    mname,
                    rname,
                    serial,
                    refresh,
                    retry,
                    expire,
                    minimum,
                },
            )
        }
        _ => unimplemented!(),
    };

    Ok((
        buffer,
        Record::new(name.clone(), qtype, qclass, ttl, rd_length, rdata),
    ))
}

impl<'a> DeSerialize<'a> for Record {
    type Item = (&'a mut Buffer<'a>, Record);

    fn deserialize(buffer: &'a mut Buffer<'a>) -> Result<Self::Item, anyhow::Error> {
        let (buf, record) = parse_record(buffer.current, buffer.source)
            .finish()
            .map_err(|e| {
                anyhow::Error::msg(format!("Error at: {:?}, with code: {:?}", e.input, e.code))
            })?;
        buffer.current = buf;
        Ok((buffer, record))
    }
}

impl Display for Record {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}\t\t\t{}\t{}\t{}\t{}",
            self.name,
            self.ttl.as_secs(),
            self.qclass,
            self.qtype,
            self.rdata
        )
    }
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn parse_record() {
        let raw = vec![
            0x06, 0x67, 0x6f, 0x6f, 0x67, 0x6c, 0x65, 0x03, 0x63, 0x6f, 0x6d, 0x00, 0x00, 0x01,
            0x00, 0x01, 0x00, 0x00, 0x0e, 0x10, 0x00, 0x04, 0x01, 0x02, 0x03, 0x04,
        ];

        let mut buffer = Buffer {
            current: &raw,
            source: &raw,
        };
        let (_, actual) = Record::deserialize(&mut buffer).unwrap();

        let expected = Record::new(
            "google.com".to_owned(),
            QType::A,
            QClass::IN,
            Duration::new(3600, 0),
            4,
            RData::A(Ipv4Addr::new(1, 2, 3, 4)),
        );

        assert_eq!(expected, actual);
    }
}
