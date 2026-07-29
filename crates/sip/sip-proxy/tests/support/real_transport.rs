use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};

pub const IO_TIMEOUT: Duration = Duration::from_secs(5);

// A loopback-only self-signed certificate whose SANs are `localhost` and
// `127.0.0.1`. It is test data, not a credential. Keeping the pair inline
// makes the SIPS test hermetic without adding a certificate-generation
// dependency to the production proxy crate.
const TEST_CA_CERTIFICATE_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDHzCCAgegAwIBAgIUcsrtKQwvYHcaUQ9F1Yt+BRqyxxYwDQYJKoZIhvcNAQEL
BQAwHjEcMBoGA1UEAwwTcnZvaXAgcHJveHkgdGVzdCBDQTAgFw0yNjA3MjYyMjMw
MzFaGA8yMTI2MDcwMjIyMzAzMVowHjEcMBoGA1UEAwwTcnZvaXAgcHJveHkgdGVz
dCBDQTCCASIwDQYJKoZIhvcNAQEBBQADggEPADCCAQoCggEBAJUho2VqFxXNr3Au
yc43X2236TBjhGQs3eGSSHMuy+YYYkfbuXggqS0a7Sal5elJDd4cxOdNyVn/G3JG
+q24+8es/zFZ3gyKo0MpwJv2rQkI3fYn+MwGVI5IMVMOuxUk0mTfYVODzA4jdCiF
T1AP8EUO2v06lm3fLw4p1Df9mQX9fpTPLtMYoFZ2Fl2P41pev+pamzoM3qGzZ7/x
QN9wPdPtfZDiTpQ+LGVkYnDRjcAfwjK+b/AWplkEBqqeXh22hMq1ToRmIsBxZNu6
kHMdMaWJfl0RnFYQ1B//p9CdPHqhJEY3Bivq5yYVjDiu8rjsV/rV7cTJUjwF6U0N
8uZULx0CAwEAAaNTMFEwHQYDVR0OBBYEFJaXeJDyUIAeJGgWRKfQ+oXAc9zuMB8G
A1UdIwQYMBaAFJaXeJDyUIAeJGgWRKfQ+oXAc9zuMA8GA1UdEwEB/wQFMAMBAf8w
DQYJKoZIhvcNAQELBQADggEBAG4ShurUhSXZQlqaURLMVLVVrcCLUNM8p8fpOD9W
eZ6pt9Mu4QNNs6dHJtm92ymO1RoZGWx8T44s/PBCSgxY83DHWGreDfMIbjNYc//y
q9QKsb+tQhTBwNypsfFCSpf9hDaAAqXYbivNHl07+Jc+dZSNARvaMSvpvuJ3Oday
HM5nRODwSR9g4cmsvFzgraPbD9Ve/PxizclehX21Gj+AU++Ukrg81Hhp+4pHfv+/
yicesdAX+uiK0DWn8uka6JaxrsNU7F/TWmSdRAgdnFBg0UNvriU3mue78e6yLrN+
zQc2ArEycuucXWumz77Zxqv+aES2VeXvGXueeP7eX3QyUHI=
-----END CERTIFICATE-----
"#;

const TEST_CERTIFICATE_PEM: &str = r#"-----BEGIN CERTIFICATE-----
MIIDVTCCAj2gAwIBAgIUT/tmTnc9TSHLiuAcHME8pqmfI2IwDQYJKoZIhvcNAQEL
BQAwHjEcMBoGA1UEAwwTcnZvaXAgcHJveHkgdGVzdCBDQTAgFw0yNjA3MjYyMjMw
MzFaGA8yMTI2MDcwMjIyMzAzMVowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjAN
BgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEA2nq4+C0+PrCz+SGMMK3gdqRh6175
HBhpH5IDZ1JCXDuOgq2GajKf1al8WVvFJqQssa0RuGOWbUCWtq/xJrtWpMQSXi/U
dBrBqvXik3t/tHzoApU6GxJBR5MP4sRv2qnBIF2KWqL81woDBIyHfqS5EJ3z2K8o
59xFGL9NmO8P/c8+9y7oXKJPbtSqoQuFzHK6sfM65iEuF/0Vl5jWTHGVOYbaBaw1
cwIP1PI4hT0Edv7Lagx6cImlNQHTgLui64ht/vE/fsC/0/+W+wAaKbsjRUMag5zC
Oqa5GAUtB1JgCr6ib+yABuAgZuDFzCebRtlxpDOzLlQMPOiYTyitvcK0CQIDAQAB
o4GSMIGPMBoGA1UdEQQTMBGCCWxvY2FsaG9zdIcEfwAAATAMBgNVHRMBAf8EAjAA
MA4GA1UdDwEB/wQEAwIFoDATBgNVHSUEDDAKBggrBgEFBQcDATAdBgNVHQ4EFgQU
olexvI2hzPobAtv7eilxDhjeF3cwHwYDVR0jBBgwFoAUlpd4kPJQgB4kaBZEp9D6
hcBz3O4wDQYJKoZIhvcNAQELBQADggEBABVPRfiNOGRu/7/jDM0weEZNP7uvV1JU
eKI7UUVfFc2ThGqYBU8CftWmGrkkEsIafyO1TJoIQDaXbB+FJyBbJxCmWPjGx8jb
MbZbU2Yz3TGgi8b6CfMG6VOMEUBRz03WyWJcTbLiXtiQk+qfbYI0t4fvihgh/XM1
QCU5BDBt9vOrJFBgR7RBAgdLNfjyOQDZc9nkA+U4Swv39vn+L0WhARNjX2LBqjwj
7TOOKLaLKa0jRJZBx4IDbGPT0+q1XbV4xJSqxZDi5djwWHHSIzAxmnHguAHSw59A
vCFG0bPtptX0CSOZXxqRvn7AiQO/5roXSFP+kYPqj1t/eAjcn+2c6oI=
-----END CERTIFICATE-----
"#;

const TEST_PRIVATE_KEY_PEM: &str = r#"-----BEGIN PRIVATE KEY-----
MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQDaerj4LT4+sLP5
IYwwreB2pGHrXvkcGGkfkgNnUkJcO46CrYZqMp/VqXxZW8UmpCyxrRG4Y5ZtQJa2
r/Emu1akxBJeL9R0GsGq9eKTe3+0fOgClTobEkFHkw/ixG/aqcEgXYpaovzXCgME
jId+pLkQnfPYryjn3EUYv02Y7w/9zz73Luhcok9u1KqhC4XMcrqx8zrmIS4X/RWX
mNZMcZU5htoFrDVzAg/U8jiFPQR2/stqDHpwiaU1AdOAu6LriG3+8T9+wL/T/5b7
ABopuyNFQxqDnMI6prkYBS0HUmAKvqJv7IAG4CBm4MXMJ5tG2XGkM7MuVAw86JhP
KK29wrQJAgMBAAECggEAAJTAYC/MN66qbXh+3tqsVNrY3dJ88OuwGXx+1AWefCm2
YwRi7dTG1j+JcqCXwVbao/nujzuu+NYDcJnxylh7AWlgfFQ+CU8DtXIw95M/aQBg
cTCENoueYjo/md4YenYZt7EEy+GDsNWFSFWDvyNhNwX/uFOC7tbubunqiWqwjRkn
C+96eaNGGVm7nFCNRvWR/Pu8CM8YFkZ5rhalQbBA/6jjxKVYBD82FjUgv5R7d2Eh
G14Zq7MP0dWGIci2/tOB3pgN0U6V5ZV1HOD3trPlw66/wJDDHiQrbcGtCieoKVjy
C+MRXG8TQaibowlFzX7hovupueJZHmZGqnVOoqGnBwKBgQD75EUgVyJ/4fMavSm+
rZSLNtltRPqOPwYgrYll13Wf5QH38Y+zDbGUC1DPNtV2tcF9vNd3M/va97smUwOO
z135avH1CDHIHGnUrr8U2v0F49J2tb/9sagi25X1pmtX89cVFxlQhzLYO+ppaoxN
l4TrvTalem/YYyQmJeamVXnOMwKBgQDeCvH9mzzt6tVIJXFI8K4WlJ+UI2KzktCK
oZ1ArFcJ31TQjGIJF5PZUXd4bCy8hEshLbXpPYjEqguJVGzDRP4bIWbRymsgAWHq
7bJFY0nW6SGSKHZGXa93Wq73ALShRZmSSv94fgIKuaDjJV/o1F2U74H11wguo99f
5geVOutA0wKBgQCtpUiOKeNrm10W0s7DVzAu5GnxLPs5MnNL9bXhUi4RQzMfNRSm
D8uaTk6v+pIfmt6/in5S+7Ak3GDU46dxPL0A41vXWoXO+N9wMeMiQnDpLYv6MAMh
peZN2WjAhrA7WqqsLFdUL0+6x1sqvrvoBYspZDAW1Zfi6T2TWs9tXUFyBwKBgQC7
QDe7S4MikPu0j/7tKCtn14aMAxtlnNZJUumudKgiJzj7dqfmSv/gMRezcmZ3xIkn
Pck/HSmN0GlSMuRV+ITilFSFb1LP9tqAqFvCsGzA1HH/NCgqRy+GU+9hVjL+HhfY
i27OSlWmfcz0QbyUOSOCSkkq7WB1FLV3xiF28+0ZCQKBgQDQ+K9/xsZDYRk+GZrh
eDyi9w6qq0YL2b4RvQl0XlGDtXzKP1ZpoJrhQSLmrCiY0Jr9AmL6sWVH2XLfeQhy
7RTv+502LFbl2XBZ5WqBNYt2ry84CYIu3PILSUpbfMqZek34HENoiQtVcWHU6T24
rahb7aNGNaFsg4DkFYIarFX63A==
-----END PRIVATE KEY-----
"#;

pub struct TestCertificateFiles {
    directory: PathBuf,
    ca_certificate: PathBuf,
    certificate: PathBuf,
    private_key: PathBuf,
}

impl TestCertificateFiles {
    pub fn create() -> Self {
        let unique = format!(
            "rvoip-proxy-sips-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        );
        let directory = std::env::temp_dir().join(unique);
        std::fs::create_dir(&directory).expect("create certificate test directory");
        let ca_certificate = directory.join("loopback-ca.pem");
        let certificate = directory.join("loopback-cert.pem");
        let private_key = directory.join("loopback-key.pem");
        std::fs::write(&ca_certificate, TEST_CA_CERTIFICATE_PEM)
            .expect("write loopback CA certificate");
        std::fs::write(&certificate, TEST_CERTIFICATE_PEM).expect("write loopback certificate");
        std::fs::write(&private_key, TEST_PRIVATE_KEY_PEM).expect("write loopback private key");
        Self {
            directory,
            ca_certificate,
            certificate,
            private_key,
        }
    }

    pub fn ca_certificate(&self) -> &Path {
        &self.ca_certificate
    }

    pub fn certificate(&self) -> &Path {
        &self.certificate
    }

    pub fn private_key(&self) -> &Path {
        &self.private_key
    }
}

impl Drop for TestCertificateFiles {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

pub fn request_wire(
    method: &str,
    request_uri: &str,
    transport: &str,
    sent_by: SocketAddr,
    branch: &str,
    call_id: &str,
    cseq: u32,
    max_forwards: u32,
    body: &[u8],
) -> Vec<u8> {
    let content_type = if body.is_empty() {
        String::new()
    } else {
        "Content-Type: application/sdp\r\n".to_owned()
    };
    format!(
        "{method} {request_uri} SIP/2.0\r\n\
         Via: SIP/2.0/{transport} {sent_by};branch={branch};rport\r\n\
         From: \"Alice\" <sip:alice@example.test>;tag=alice-tag\r\n\
         To: \"Bob\" <sip:bob@example.test>\r\n\
         Call-ID: {call_id}\r\n\
         CSeq: {cseq} {method}\r\n\
         Contact: <sip:alice@{sent_by}>\r\n\
         Max-Forwards: {max_forwards}\r\n\
         {content_type}Content-Length: {}\r\n\
         \r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect()
}

pub fn response_wire(
    request: &[u8],
    status: u16,
    reason: &str,
    to_tag: &str,
    body: &[u8],
) -> Vec<u8> {
    let mut lines = header_lines(request);
    let mut response = format!("SIP/2.0 {status} {reason}\r\n").into_bytes();

    for (name, value) in lines
        .drain(..)
        .filter(|(name, _)| name.eq_ignore_ascii_case("via"))
    {
        response.extend_from_slice(name.as_bytes());
        response.extend_from_slice(b": ");
        response.extend_from_slice(value.as_bytes());
        response.extend_from_slice(b"\r\n");
    }

    for wanted in ["from", "to", "call-id", "cseq"] {
        let (_, value) = header_lines(request)
            .into_iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .unwrap_or_else(|| panic!("request missing {wanted}"));
        let wire_name = match wanted {
            "from" => "From",
            "to" => "To",
            "call-id" => "Call-ID",
            "cseq" => "CSeq",
            _ => unreachable!(),
        };
        response.extend_from_slice(wire_name.as_bytes());
        response.extend_from_slice(b": ");
        response.extend_from_slice(value.as_bytes());
        if wanted == "to" && !value.to_ascii_lowercase().contains(";tag=") {
            response.extend_from_slice(format!(";tag={to_tag}").as_bytes());
        }
        response.extend_from_slice(b"\r\n");
    }

    if !body.is_empty() {
        response.extend_from_slice(b"Content-Type: application/problem+sip\r\n");
    }
    response.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes());
    response.extend_from_slice(body);
    response
}

pub fn header_lines(message: &[u8]) -> Vec<(String, String)> {
    let header_end = find_bytes(message, b"\r\n\r\n").expect("complete SIP header");
    String::from_utf8_lossy(&message[..header_end])
        .split("\r\n")
        .skip(1)
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect()
}

pub fn header_values(message: &[u8], wanted: &str) -> Vec<String> {
    header_lines(message)
        .into_iter()
        .filter_map(|(name, value)| name.eq_ignore_ascii_case(wanted).then_some(value))
        .collect()
}

pub fn body(message: &[u8]) -> &[u8] {
    let header_end = find_bytes(message, b"\r\n\r\n").expect("complete SIP header");
    &message[header_end + 4..]
}

pub fn declared_content_length(message: &[u8]) -> usize {
    header_values(message, "content-length")
        .into_iter()
        .next()
        .expect("Content-Length")
        .parse()
        .expect("numeric Content-Length")
}

pub fn start_line(message: &[u8]) -> String {
    let end = find_bytes(message, b"\r\n").expect("SIP start-line terminator");
    String::from_utf8_lossy(&message[..end]).into_owned()
}

pub async fn udp_send(socket: &UdpSocket, message: &[u8], destination: SocketAddr) {
    let written = tokio::time::timeout(IO_TIMEOUT, socket.send_to(message, destination))
        .await
        .expect("UDP send timeout")
        .expect("UDP send");
    assert_eq!(written, message.len(), "UDP datagram was truncated on send");
}

pub async fn udp_recv(socket: &UdpSocket) -> (Vec<u8>, SocketAddr) {
    let mut buffer = vec![0_u8; 65_535];
    let (length, source) = tokio::time::timeout(IO_TIMEOUT, socket.recv_from(&mut buffer))
        .await
        .expect("UDP receive timeout")
        .expect("UDP receive");
    buffer.truncate(length);
    (buffer, source)
}

pub async fn udp_expect_quiet(socket: &UdpSocket, duration: Duration) {
    let mut buffer = vec![0_u8; 65_535];
    assert!(
        tokio::time::timeout(duration, socket.recv_from(&mut buffer))
            .await
            .is_err(),
        "unexpected UDP datagram arrived"
    );
}

pub struct SipTcpPeer {
    stream: TcpStream,
    buffered: Vec<u8>,
}

impl SipTcpPeer {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            stream,
            buffered: Vec::new(),
        }
    }

    pub async fn connect(destination: SocketAddr) -> Self {
        let stream = tokio::time::timeout(IO_TIMEOUT, TcpStream::connect(destination))
            .await
            .expect("TCP connect timeout")
            .expect("TCP connect");
        Self::new(stream)
    }

    pub async fn write_message(&mut self, message: &[u8]) {
        tokio::time::timeout(IO_TIMEOUT, self.stream.write_all(message))
            .await
            .expect("TCP write timeout")
            .expect("TCP write");
    }

    pub async fn read_message(&mut self) -> Vec<u8> {
        loop {
            if let Some(length) = complete_message_length(&self.buffered) {
                return self.buffered.drain(..length).collect();
            }

            let mut chunk = [0_u8; 4096];
            let read = tokio::time::timeout(IO_TIMEOUT, self.stream.read(&mut chunk))
                .await
                .expect("TCP read timeout")
                .expect("TCP read");
            assert_ne!(read, 0, "TCP peer closed before a complete SIP message");
            self.buffered.extend_from_slice(&chunk[..read]);
        }
    }
}

fn complete_message_length(buffer: &[u8]) -> Option<usize> {
    let header_end = find_bytes(buffer, b"\r\n\r\n")?;
    let header_bytes = &buffer[..header_end];
    let content_length = String::from_utf8_lossy(header_bytes)
        .split("\r\n")
        .filter_map(|line| line.split_once(':'))
        .find_map(|(name, value)| {
            name.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .unwrap_or(0);
    let total = header_end + 4 + content_length;
    (buffer.len() >= total).then_some(total)
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
