"""Signs and checks the file table of a 0.26g resources.db (patch 24).

The game checks RSA PKCS#1 v1.5 over the SHA-1 of the file table, the bytes
[24, 24 + the u32 at offset 20) after the header; the 256 signature bytes
follow the table, least significant byte first (CryptoAPI's order). The exe
of patch revision 12 checks them with Skhid's key alone
(resources_db_key_026g.der, embedded in db_signature_026g.section.bin). RSA
is OpenSSL's: `openssl` on PATH (srp-026g/flake.nix has it).

    python sign_resources_026g.py sign <resources.db or head> [--key=PATH]
        writes the 256 bytes after the table, over zeros or a signature
    python sign_resources_026g.py verify <resources.db or head>
        "skhid" (exit 0), "retail" (exit 2) or neither (exit 1)
    python sign_resources_026g.py genkey --key=PATH --public=PATH
        a new 2048-bit key: a new key is a new .bin and a new revision

The private key is never in the repository; its default place is DEFAULT_KEY.
"""

import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OWN_KEY = os.path.join(HERE, "resources_db_key_026g.der")
DEFAULT_KEY = os.path.expanduser("~/.local/share/skhid-signing/resources-db-key.pem")
SIGNATURE_SIZE = 256
# The survarium.com certificate's key (DER RSAPublicKey), from the original
# exe's Authenticode signature: what the original game checks the table with.
RETAIL_KEY = bytes.fromhex(
    "3082010a0282010100c67f91191bdba046c5e5e5f7f301e660afea791d6158c86eac3cf4136cff1b"
    "9b7955a69c1cd7f319716a3c689dd4baefe0e80f32d94bacfc8ba4f8294eb6e889c9242ef259beaf"
    "c60adeaea1e5eb4ae365dff183f6f0e9d3b643918b4f119b717db8dd661dcd5ed7b4fd60b865605c"
    "6ad5f004f924c3daa022782d6cf7da3c625fe4fe0e9d05a2b6d6062bb86aa829897c19f165b9638d"
    "346797443763f6125bff8c6c1d679a51a702f5d3d41fc3e7333213194c9c9c5f817add8b5810e499"
    "952e7b524480e932cdc41a17e0ba52aa9e1945b6c16d4412f76aa1d242041eb684d1b9a3990f78cb"
    "3086f86e64b5c5b7554807e8b73493e1b0fbf3a70dad032e190203010001")


def openssl(*args, data=b""):
    return subprocess.run(["openssl", *args], input=data, capture_output=True)


def write(path, data):
    with open(path, "wb") as out:
        out.write(data)


def read_table(handle):
    """(header, table, signature) of an open resources.db, or of its head."""
    handle.seek(0)
    header = handle.read(24)
    if header[:14] != b"little-endian\0":
        raise ValueError("not a 0.26g resources.db")
    table = handle.read(int.from_bytes(header[20:24], "little"))
    signature = handle.read(SIGNATURE_SIZE)
    if len(signature) != SIGNATURE_SIZE:
        raise ValueError("the file ends inside the table or its signature")
    return header, table, signature


def verifies(table, signature, key_der):
    with tempfile.TemporaryDirectory() as folder:
        key, sig = os.path.join(folder, "key.der"), os.path.join(folder, "sig")
        write(key, key_der)
        write(sig, signature[::-1])
        return openssl("dgst", "-sha1", "-keyform", "DER", "-verify", key, "-signature", sig,
                       data=table).returncode == 0


def which_key(table, signature):
    """"skhid", "retail" or None: the key that signed the table."""
    if verifies(table, signature, open(OWN_KEY, "rb").read()):
        return "skhid"
    return "retail" if verifies(table, signature, RETAIL_KEY) else None


def sign_file(path, key=DEFAULT_KEY):
    """Signs the table of the resources.db (or head) at `path` in place."""
    with open(path, "r+b") as handle:
        header, table, old = read_table(handle)
        if any(old) and which_key(table, old) is None:
            raise ValueError("the 256 bytes after the table are neither zeros nor its signature")
        out = openssl("dgst", "-sha1", "-sign", key, data=table)
        if out.returncode or len(out.stdout) != SIGNATURE_SIZE:
            raise ValueError(f"openssl could not sign with {key}: {out.stderr.decode(errors='replace')}")
        handle.seek(24 + len(table))
        handle.write(out.stdout[::-1])


def genkey(key, public):
    for path in (key, public):
        if os.path.exists(path):
            sys.exit(f"{path} exists; not overwritten")
    os.makedirs(os.path.dirname(os.path.abspath(key)), mode=0o700, exist_ok=True)
    pem = openssl("genrsa", "2048").stdout
    with os.fdopen(os.open(key, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), "wb") as out:
        out.write(pem)
    write(public, openssl("rsa", "-in", key, "-RSAPublicKey_out", "-outform", "DER").stdout)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    options = dict(a[2:].split("=", 1) for a in sys.argv[1:] if a.startswith("--") and "=" in a)
    if args[:1] == ["sign"] and len(args) == 2:
        sign_file(args[1], options.get("key", DEFAULT_KEY))
        print(f"{args[1]}: signed with {options.get('key', DEFAULT_KEY)}")
    elif args[:1] == ["verify"] and len(args) == 2:
        with open(args[1], "rb") as handle:
            signer = which_key(*read_table(handle)[1:])
        print(f"{args[1]}: {signer or 'neither key'}")
        sys.exit({"skhid": 0, "retail": 2, None: 1}[signer])
    elif args == ["genkey"] and {"key", "public"} <= options.keys():
        genkey(options["key"], options["public"])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
