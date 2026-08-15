#!/usr/bin/env sh
set -eu

FS_HOME=${FS_HOME:-/usr/local/freeswitch}
CONF_DIR="${FS_CONF_DIR:-$FS_HOME/etc/freeswitch}"

value_or_container_ip() {
  value=$1
  if [ "$value" != "auto" ] && [ -n "$value" ]; then
    printf "%s\n" "$value"
    return
  fi

  hostname -i 2>/dev/null | awk '{print $1; exit}'
}

set_xml_var() {
  name=$1
  value=$2
  file="$CONF_DIR/vars.xml"
  if grep -q "data=\"$name=" "$file"; then
    sed -i "s#data=\"$name=[^\"]*\"#data=\"$name=$value\"#g" "$file"
  else
    sed -i "/<\/include>/i\\  <X-PRE-PROCESS cmd=\"set\" data=\"$name=$value\"/>" "$file"
  fi
}

set_switch_param() {
  name=$1
  value=$2
  file="$CONF_DIR/autoload_configs/switch.conf.xml"
  if grep -q "^[[:space:]]*<param name=\"$name\"" "$file"; then
    sed -i "s#^[[:space:]]*<param name=\"$name\" value=\"[^\"]*\"/>#    <param name=\"$name\" value=\"$value\"/>#g" "$file"
  else
    sed -i "/<settings>/a\\    <param name=\"$name\" value=\"$value\"/>" "$file"
  fi
}

set_profile_param() {
  name=$1
  value=$2
  file="$CONF_DIR/sip_profiles/internal.xml"
  if grep -q "name=\"$name\"" "$file"; then
    sed -i "s#<param name=\"$name\" value=\"[^\"]*\"/>#<param name=\"$name\" value=\"$value\"/>#g" "$file"
  else
    sed -i "/<settings>/a\\    <param name=\"$name\" value=\"$value\"/>" "$file"
  fi
}

ensure_tls_cert() {
  tls_dir="$CONF_DIR/tls"
  mkdir -p "$tls_dir"

  if [ -s "$tls_dir/agent.pem" ] && [ -s "$tls_dir/cafile.pem" ]; then
    return 0
  fi

  tmp_key="$tls_dir/rvoip-agent.key"
  tmp_crt="$tls_dir/rvoip-agent.crt"
  openssl req -x509 -newkey rsa:2048 -nodes \
    -keyout "$tmp_key" \
    -out "$tmp_crt" \
    -days "${FS_TLS_CERT_DAYS:-3650}" \
    -subj "/CN=rvoip-freeswitch.local" \
    -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" >/dev/null 2>&1
  cat "$tmp_crt" "$tmp_key" > "$tls_dir/agent.pem"
  cp "$tmp_crt" "$tls_dir/cafile.pem"
}

write_rvoip_profile() {
  name=$1
  sip_port=$2
  tls_enabled=$3
  tls_only=$4
  secure_media=$5
  tls_port=$6
  local_rtp_ip=$7
  external_sip_ip=$8
  external_rtp_ip=$9
  # "true" pins disable-transcoding, which REFUSES a call whose legs cannot
  # share a codec. That is what the matrix wants almost everywhere -- a row
  # that quietly transcodes proves nothing about codec negotiation -- but the
  # amr_transcode_call row needs the opposite, so it registers against the
  # _xcode twins written below.
  disable_transcoding=${10:-true}
  secure_media_line=""
  if [ "$secure_media" = "true" ]; then
    secure_media_line='    <param name="rtp_secure_media" value="mandatory"/>'
  fi

  cat > "$CONF_DIR/sip_profiles/$name.xml" <<EOF
<profile name="$name">
  <aliases/>
  <gateways/>
  <domains>
    <domain name="all" alias="true" parse="false"/>
  </domains>
  <settings>
    <param name="debug" value="0"/>
    <param name="sip-trace" value="no"/>
    <param name="sip-capture" value="no"/>
    <param name="watchdog-enabled" value="no"/>
    <param name="context" value="rvoip"/>
    <param name="dialplan" value="XML"/>
    <param name="rfc2833-pt" value="101"/>
    <param name="dtmf-duration" value="2000"/>
    <param name="sip-port" value="$sip_port"/>
    <param name="sip-ip" value="$sip_ip"/>
    <param name="rtp-ip" value="$local_rtp_ip"/>
    <param name="ext-sip-ip" value="$external_sip_ip"/>
    <param name="ext-rtp-ip" value="$external_rtp_ip"/>
    <param name="inbound-codec-prefs" value="AMR-WB,AMR,G729,PCMU,PCMA"/>
    <param name="outbound-codec-prefs" value="AMR-WB,AMR,G729,PCMU,PCMA"/>
    <param name="inbound-late-negotiation" value="true"/>
    <param name="disable-transcoding" value="$disable_transcoding"/>
    <param name="rtp-timer-name" value="soft"/>
    <param name="auth-calls" value="true"/>
    <param name="auth-subscriptions" value="false"/>
    <param name="auth-all-packets" value="false"/>
    <param name="inbound-reg-force-matching-username" value="true"/>
    <param name="force-register-domain" value="\$\${domain}"/>
    <param name="force-register-db-domain" value="\$\${domain}"/>
    <param name="challenge-realm" value="auto_from"/>
    <param name="nonce-ttl" value="60"/>
    <param name="manage-presence" value="false"/>
${secure_media_line}
    <param name="tls" value="$tls_enabled"/>
    <param name="tls-only" value="$tls_only"/>
    <param name="tls-bind-params" value="transport=tls"/>
    <param name="tls-sip-port" value="$tls_port"/>
    <param name="tls-cert-dir" value="$CONF_DIR/tls"/>
    <param name="tls-passphrase" value=""/>
    <param name="tls-verify-date" value="false"/>
    <param name="tls-verify-policy" value="none"/>
    <param name="tls-verify-depth" value="2"/>
    <param name="tls-version" value="\$\${sip_tls_version}"/>
    <param name="tls-ciphers" value="\$\${sip_tls_ciphers}"/>
  </settings>
</profile>
EOF
}

write_rvoip_profiles() {
  local_rtp_ip=$1
  external_sip_ip=$2
  external_rtp_ip=$3
  udp_port=${FS_RVOIP_UDP_SIP_PORT:-5062}
  tls_port=${FS_RVOIP_TLS_SIP_PORT:-5063}
  udp_xcode_port=${FS_RVOIP_UDP_XCODE_SIP_PORT:-5064}
  tls_xcode_port=${FS_RVOIP_TLS_XCODE_SIP_PORT:-5065}

  write_rvoip_profile rvoip_udp "$udp_port" false false false "$tls_port" \
    "$local_rtp_ip" "$external_sip_ip" "$external_rtp_ip" true
  write_rvoip_profile rvoip_tls_srtp "$tls_port" true true true "$tls_port" \
    "$local_rtp_ip" "$external_sip_ip" "$external_rtp_ip" true

  # Transcoding twins, on their own ports.
  #
  # amr_transcode_call bridges two legs that deliberately share no codec
  # (AMR-NB one side, PCMU the other) and asks the PBX to convert between
  # them. The profiles above pin disable-transcoding, so such a call is
  # refused rather than converted -- correct for every other row, fatal for
  # this one. run.sh redirects that scenario to FREESWITCH_XCODE_*_ADDR, and
  # these are the profiles those addresses point at.
  write_rvoip_profile rvoip_udp_xcode "$udp_xcode_port" false false false \
    "$tls_xcode_port" "$local_rtp_ip" "$external_sip_ip" "$external_rtp_ip" false
  write_rvoip_profile rvoip_tls_srtp_xcode "$tls_xcode_port" true true true \
    "$tls_xcode_port" "$local_rtp_ip" "$external_sip_ip" "$external_rtp_ip" false
}

write_rvoip_user() {
  user=$1
  secure_media=$2
  password=<redacted>
  secure_line=""
  if [ "$secure_media" = "true" ]; then
    secure_line='      <variable name="rtp_secure_media" value="mandatory"/>'
  fi

  cat > "$CONF_DIR/directory/default/$user.xml" <<EOF
<include>
  <user id="$user">
    <params>
      <param name="password" value="$password"/>
      <param name="vm-password" value="$user"/>
    </params>
    <variables>
      <variable name="toll_allow" value="local"/>
      <variable name="accountcode" value="$user"/>
      <variable name="user_context" value="rvoip"/>
      <variable name="effective_caller_id_name" value="rvoip $user"/>
      <variable name="effective_caller_id_number" value="$user"/>
$secure_line
    </variables>
  </user>
</include>
EOF
}

write_rvoip_users() {
  rm -f "$CONF_DIR/directory/default/1004.xml" "$CONF_DIR/directory/default/2004.xml"

  for user in 1001 1002 1003; do
    write_rvoip_user "$user" true
  done
  for user in 2001 2002 2003; do
    write_rvoip_user "$user" false
  done
}

write_rvoip_dialplan() {
  cat > "$CONF_DIR/dialplan/rvoip.xml" <<'EOF'
<include>
  <context name="rvoip">
    <extension name="rvoip_tls_srtp_extensions">
      <condition field="destination_number" expression="^(100[1-3])$">
        <action application="set" data="dialed_extension=$1"/>
        <action application="set" data="ringback=${us-ring}"/>
        <action application="set" data="transfer_ringback=$${hold_music}"/>
        <action application="set" data="call_timeout=30"/>
        <action application="set" data="hangup_after_bridge=true"/>
        <action application="set" data="continue_on_fail=false"/>
        <action application="set" data="rtp_secure_media=mandatory"/>
        <action application="export" data="rtp_secure_media=mandatory"/>
        <action application="bridge" data="user/${dialed_extension}@${domain_name}"/>
      </condition>
    </extension>

    <extension name="rvoip_udp_extensions">
      <condition field="destination_number" expression="^(200[1-3])$">
        <action application="set" data="dialed_extension=$1"/>
        <action application="set" data="ringback=${us-ring}"/>
        <action application="set" data="transfer_ringback=$${hold_music}"/>
        <action application="set" data="call_timeout=30"/>
        <action application="set" data="hangup_after_bridge=true"/>
        <action application="set" data="continue_on_fail=false"/>
        <action application="bridge" data="user/${dialed_extension}@${domain_name}"/>
      </condition>
    </extension>
  </context>
</include>
EOF
}

write_modules_conf() {
  cat > "$CONF_DIR/autoload_configs/modules.conf.xml" <<'EOF'
<configuration name="modules.conf" description="Modules">
  <modules>
    <load module="mod_console"/>
    <load module="mod_logfile"/>
    <load module="mod_event_socket"/>
    <load module="mod_sofia"/>
    <load module="mod_loopback"/>
    <load module="mod_commands"/>
    <load module="mod_conference"/>
    <load module="mod_db"/>
    <load module="mod_dptools"/>
    <load module="mod_expr"/>
    <load module="mod_fifo"/>
    <load module="mod_hash"/>
    <load module="mod_voicemail"/>
    <load module="mod_dialplan_xml"/>
    <load module="mod_sndfile"/>
    <load module="mod_native_file"/>
    <load module="mod_local_stream"/>
    <load module="mod_say_en"/>
    <load module="mod_xml_rpc"/>
    <load module="mod_g729"/>
    <!-- Built and installed by the image (modules.conf lists codecs/mod_amr
         and codecs/mod_amrwb, and the builder carries the opencore/vo-amrwbenc
         dev packages), but this file replaces FreeSWITCH's sample autoload
         list wholesale — so a codec missing here is a codec the running
         switch does not have, however well it compiled. Leaving them out is
         what made `show codec` report neither, and the AMR interop rows fail
         against a PBX that could not have answered them. -->
    <load module="mod_amr"/>
    <load module="mod_amrwb"/>
  </modules>
</configuration>
EOF
}

sip_ip=${FS_SIP_IP:-0.0.0.0}
external_sip_ip=$(value_or_container_ip "${FS_EXTERNAL_SIP_IP:-auto}")
external_rtp_ip=$(value_or_container_ip "${FS_EXTERNAL_RTP_IP:-auto}")
local_ip=$(value_or_container_ip "${FS_LOCAL_RTP_IP:-auto}")

write_modules_conf
ensure_tls_cert

set_xml_var default_password "${FS_DEFAULT_PASSWORD:<redacted>
set_xml_var external_sip_ip "$external_sip_ip"
set_xml_var external_rtp_ip "$external_rtp_ip"
set_xml_var bind_server_ip 0.0.0.0
set_xml_var internal_sip_port 5060
set_xml_var rvoip_udp_sip_port "${FS_RVOIP_UDP_SIP_PORT:-5062}"
set_xml_var rvoip_tls_sip_port "${FS_RVOIP_TLS_SIP_PORT:-5063}"

set_switch_param rtp-start-port "${FS_RTP_START:-16384}"
set_switch_param rtp-end-port "${FS_RTP_END:-16484}"

set_profile_param sip-ip "$sip_ip"
set_profile_param ext-sip-ip "$external_sip_ip"
set_profile_param ext-rtp-ip "$external_rtp_ip"

write_rvoip_users
write_rvoip_dialplan
write_rvoip_profiles "$local_ip" "$external_sip_ip" "$external_rtp_ip"

event_socket="$CONF_DIR/autoload_configs/event_socket.conf.xml"
if [ -f "$event_socket" ]; then
  sed -i "s#<param name=\"password\" value=\"[^\"]*\"/>#<param name=\"password\" value=\"${FS_EVENT_SOCKET_PASSWORD:<redacted>
  sed -i "s#<param name=\"listen-ip\" value=\"[^\"]*\"/>#<param name=\"listen-ip\" value=\"127.0.0.1\"/>#g" "$event_socket"
fi

exec "$@"
