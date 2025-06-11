#!/usr/bin/env python3
"""
Create a proper PCAP file with G.711A RTP packets for SIPp testing.
This creates real RTP packets that SIPp can play back.
"""

import struct
import time
import wave

def create_pcap_header():
    """Create PCAP global header"""
    return struct.pack('<LHHLLLL',
        0xa1b2c3d4,  # magic number (microsecond precision)
        2,           # version major
        4,           # version minor  
        0,           # thiszone
        0,           # sigfigs
        65535,       # snaplen
        1            # network (Ethernet)
    )

def create_ethernet_header():
    """Create Ethernet header"""
    # Destination MAC (6 bytes) + Source MAC (6 bytes) + EtherType (2 bytes)
    return (b'\x00\x01\x02\x03\x04\x05' +  # dst MAC
            b'\x00\x0a\x0b\x0c\x0d\x0e' +  # src MAC  
            b'\x08\x00')                     # IP EtherType

def create_ip_header(payload_length):
    """Create IP header"""
    total_length = 20 + 8 + 12 + payload_length  # IP + UDP + RTP + payload
    return struct.pack('!BBHHHBBH4s4s',
        0x45,                    # version (4) + IHL (5)
        0,                       # DSCP + ECN
        total_length,            # total length
        0x1234,                  # identification
        0,                       # flags + fragment offset
        64,                      # TTL
        17,                      # protocol (UDP)
        0,                       # checksum (will be 0)
        b'\x7f\x00\x00\x01',   # source IP (127.0.0.1)
        b'\x7f\x00\x00\x01'    # dest IP (127.0.0.1)
    )

def create_udp_header(payload_length):
    """Create UDP header"""
    udp_length = 8 + 12 + payload_length  # UDP + RTP + payload
    return struct.pack('!HHHH',
        12345,       # source port
        6000,        # destination port (SIPp RTP port)
        udp_length,  # length
        0            # checksum
    )

def create_rtp_header(seq_num, timestamp, ssrc=0x12345678):
    """Create RTP header"""
    return struct.pack('!BBHLL',
        0x80,        # V=2, P=0, X=0, CC=0
        8,           # M=0, PT=8 (PCMA/G.711A)
        seq_num,     # sequence number
        timestamp,   # RTP timestamp
        ssrc         # SSRC
    )

def generate_g711a_audio(duration_ms=20):
    """Generate G.711A audio samples (A-law encoded)"""
    # Generate a simple 440Hz tone pattern in G.711A format
    samples_per_packet = int(8000 * duration_ms / 1000)  # 8kHz sampling rate
    
    # Simple pattern that represents a 440Hz tone in G.711A encoding
    # This is a rough approximation - real G.711A would need proper conversion
    audio_pattern = []
    for i in range(samples_per_packet):
        # Create a sine-like pattern in A-law format
        sample = int(127 * (1 + 0.8 * ((i % 18) - 9) / 9))  # Rough 440Hz at 8kHz
        # Convert to A-law (simplified - just use sample directly)
        a_law_sample = max(0, min(255, sample))
        audio_pattern.append(a_law_sample)
    
    return bytes(audio_pattern)

def create_pcap_record(packet_data, timestamp_sec, timestamp_usec):
    """Create PCAP record header + data"""
    return struct.pack('<LLLL',
        timestamp_sec,           # timestamp seconds
        timestamp_usec,          # timestamp microseconds
        len(packet_data),        # captured packet length
        len(packet_data)         # original packet length
    ) + packet_data

def main():
    """Create the RTP PCAP file"""
    print("🎵 Creating RTP PCAP file with G.711A audio...")
    
    with open('pcap/g711a.pcap', 'wb') as f:
        # Write PCAP header
        f.write(create_pcap_header())
        
        # Generate RTP packets (1 second of audio at 20ms intervals = 50 packets)
        base_timestamp = int(time.time())
        rtp_timestamp = 0
        
        for seq_num in range(50):
            # Generate G.711A audio payload
            audio_payload = generate_g711a_audio(20)  # 20ms of audio
            
            # Create packet layers
            ethernet = create_ethernet_header()
            ip = create_ip_header(len(audio_payload))
            udp = create_udp_header(len(audio_payload))
            rtp = create_rtp_header(seq_num + 1000, rtp_timestamp)
            
            # Combine all layers
            packet = ethernet + ip + udp + rtp + audio_payload
            
            # Calculate timestamp for this packet
            packet_time_usec = seq_num * 20000  # 20ms intervals in microseconds
            timestamp_sec = base_timestamp + (packet_time_usec // 1000000)
            timestamp_usec = packet_time_usec % 1000000
            
            # Write PCAP record
            record = create_pcap_record(packet, timestamp_sec, timestamp_usec)
            f.write(record)
            
            # Update RTP timestamp (160 samples per 20ms at 8kHz)
            rtp_timestamp += 160
    
    print(f"✅ Created pcap/g711a.pcap with 50 RTP packets (1 second of G.711A audio)")
    
    # Verify the file
    try:
        import subprocess
        result = subprocess.run(['tcpdump', '-r', 'pcap/g711a.pcap', '-c', '5'], 
                              capture_output=True, text=True)
        if result.returncode == 0:
            print("📊 PCAP file verification:")
            print(result.stdout)
        else:
            print("⚠️ Could not verify PCAP with tcpdump")
    except Exception:
        print("ℹ️ tcpdump not available for verification")

if __name__ == "__main__":
    main() 