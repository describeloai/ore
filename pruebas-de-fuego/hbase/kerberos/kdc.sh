#!/bin/bash
# Crea la base del reino la primera vez, las entidades y los keytab en /keytabs, y deja el KDC en primer plano.
set -e
mkdir -p /keytabs /etc/krb5kdc
printf '*/admin@ORE.TEST *\n' > /etc/krb5kdc/kadm5.acl
cat > /etc/krb5kdc/kdc.conf <<CONF
[kdcdefaults]
  kdc_ports = 88
  kdc_tcp_ports = 88
[realms]
  ORE.TEST = {
    database_name = /var/lib/krb5kdc/principal
    key_stash_file = /etc/krb5kdc/stash
    acl_file = /etc/krb5kdc/kadm5.acl
    supported_enctypes = aes256-cts-hmac-sha1-96:normal
    max_life = 10h 0m 0s
  }
CONF
if [ ! -f /var/lib/krb5kdc/principal ]; then
  kdb5_util create -s -r ORE.TEST -P maestra
  kadmin.local -q "addprinc -randkey hbase/hb.ore.test@ORE.TEST"
  kadmin.local -q "addprinc -pw lector lector@ORE.TEST"
  kadmin.local -q "addprinc -pw intruso intruso@ORE.TEST"
  kadmin.local -q "ktadd -k /keytabs/hbase.keytab hbase/hb.ore.test@ORE.TEST"
  kadmin.local -q "ktadd -k /keytabs/lector.keytab lector@ORE.TEST"
  chmod 644 /keytabs/*.keytab
fi
exec krb5kdc -n
