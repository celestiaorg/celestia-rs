#!/bin/sh
set -eu
cd "$(dirname "$0")"
mkdir -p tls
openssl req -x509 -newkey rsa:2048 -sha256 -nodes \
  -days 36500 \
  -config tls/openssl.cnf \
  -keyout tls/localhost.key \
  -out tls/localhost.crt
openssl x509 -in tls/localhost.crt -noout -text >tls/localhost-cert.txt
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048 -out tls/mismatch.key
openssl version -a >tls/openssl-version.txt
