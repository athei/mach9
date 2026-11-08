// The parts of appcast.sh that need an XML parser or Ed25519. appcast.sh
// compiles it once per run:
//
//   xcrun swiftc -O -o <tool> launcher/scripts/appcast-tool.swift
//
// Commands:
//
//   public-key
//       Reads a private EdDSA key in Sparkle's key file format on stdin (the
//       base64 of the 32-byte seed generate_keys writes, or of the older
//       96-byte private and public key) and prints the base64 public key,
//       the form SUPublicEDKey has.
//   items <appcast>
//       One line per item: sparkle:version, sparkle:shortVersionString and
//       the file name of its enclosure, separated by tabs.
//   notes <appcast> <dir> <app>
//       Writes the description of each item that has one to
//       <dir>/<app>-<short version>.<ext>, <ext> being md, txt or html after
//       the description's sparkle:format. That is the name appcast.sh gives
//       the item's DMG in its work directory, so generate_appcast embeds the
//       notes again as they were. Prints the files written.
//   enclosures <appcast>
//       One line per enclosure, deltas included: the item's short version,
//       full or delta, the file name, the sparkle:edSignature and the URL,
//       separated by tabs.
//   rewrite <appcast> <prefix> <app>
//       Sets the URL of every item's own enclosure to
//       <prefix>/v<short version>/<app>.dmg and that of every delta to
//       <prefix>/v<short version>/<file name>: each archive is an asset of
//       the release of the item it belongs to, and every release has its DMG
//       under the app's name alone. Writes the file in place.
//
// Sparkle's own elements and attributes are matched by namespace, so the
// prefix a feed binds to it does not matter.

import CryptoKit
import Foundation

let sparkleNS = "http://www.andymatuschak.org/xml-namespaces/sparkle"

func fail(_ message: String) -> Never {
    FileHandle.standardError.write(Data("appcast-tool: \(message)\n".utf8))
    exit(1)
}

func load(_ path: String) -> XMLDocument {
    let options: XMLNode.Options = [
        .nodeLoadExternalEntitiesNever, .nodePreserveCDATA, .nodePreserveWhitespace,
    ]
    do {
        return try XMLDocument(contentsOf: URL(fileURLWithPath: path), options: options)
    } catch {
        fail("cannot read \(path): \(error.localizedDescription)")
    }
}

func items(_ doc: XMLDocument) -> [XMLElement] {
    guard let root = doc.rootElement(), root.name == "rss" else {
        fail("not an appcast: the root element is not rss")
    }
    let channels = root.elements(forName: "channel")
    guard channels.count == 1 else {
        fail("not an appcast: \(channels.count) channel elements")
    }
    return channels[0].elements(forName: "item")
}

func sparkleChild(_ element: XMLElement, _ name: String) -> XMLElement? {
    element.elements(forLocalName: name, uri: sparkleNS).first
}

func sparkleAttribute(_ element: XMLElement, _ name: String) -> String? {
    element.attribute(forLocalName: name, uri: sparkleNS)?.stringValue
}

func enclosure(_ item: XMLElement) -> XMLElement {
    guard let enclosure = item.elements(forName: "enclosure").first else {
        fail("an item has no enclosure")
    }
    return enclosure
}

/// The element sparkle:version or, in older feeds, the enclosure's attribute.
func version(_ item: XMLElement) -> String {
    if let value = sparkleChild(item, "version")?.stringValue ?? sparkleAttribute(enclosure(item), "version") {
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
    fail("an item has no sparkle:version")
}

func shortVersion(_ item: XMLElement) -> String {
    if let value = sparkleChild(item, "shortVersionString")?.stringValue
        ?? sparkleAttribute(enclosure(item), "shortVersionString")
    {
        return value.trimmingCharacters(in: .whitespacesAndNewlines)
    }
    fail("item \(version(item)) has no sparkle:shortVersionString")
}

func fileName(_ enclosure: XMLElement) -> String {
    guard let url = enclosure.attribute(forName: "url")?.stringValue,
        let name = url.split(separator: "/").last, !name.isEmpty
    else {
        fail("an enclosure has no URL")
    }
    return String(name)
}

/// Every enclosure of an item: its own, then those of its deltas.
func enclosures(_ item: XMLElement) -> [(kind: String, element: XMLElement)] {
    var all = [(kind: "full", element: enclosure(item))]
    for deltas in item.elements(forLocalName: "deltas", uri: sparkleNS) {
        for delta in deltas.elements(forName: "enclosure") {
            all.append((kind: "delta", element: delta))
        }
    }
    return all
}

func publicKey() {
    let input = FileHandle.standardInput.readDataToEndOfFile()
    let text = String(decoding: input, as: UTF8.self).trimmingCharacters(in: .whitespacesAndNewlines)
    guard let secret = Data(base64Encoded: text) else {
        fail("the key on stdin is not base64")
    }
    let key: Data
    switch secret.count {
    case 32:
        guard let seed = try? Curve25519.Signing.PrivateKey(rawRepresentation: secret) else {
            fail("the key on stdin is not an Ed25519 seed")
        }
        key = seed.publicKey.rawRepresentation
    case 96:
        key = secret.suffix(32)
    default:
        fail("the key on stdin is \(secret.count) bytes, neither a 32-byte seed nor a 96-byte key")
    }
    print(key.base64EncodedString())
}

func listItems(_ path: String) {
    for item in items(load(path)) {
        print([version(item), shortVersion(item), fileName(enclosure(item))].joined(separator: "\t"))
    }
}

func writeNotes(_ path: String, _ dir: String, _ app: String) {
    for item in items(load(path)) {
        guard let description = item.elements(forName: "description").first,
            let text = description.stringValue
        else {
            continue
        }
        let ext: String
        switch sparkleAttribute(description, "format") {
        case "markdown": ext = "md"
        case "plain-text": ext = "txt"
        default: ext = "html"
        }
        // Not the enclosure's file name: every release's DMG has the same one.
        let archive = "\(app)-\(shortVersion(item))"
        let out = URL(fileURLWithPath: dir).appendingPathComponent("\(archive).\(ext)")
        do {
            try Data(text.utf8).write(to: out)
        } catch {
            fail("cannot write \(out.path): \(error.localizedDescription)")
        }
        print(out.lastPathComponent)
    }
}

func listEnclosures(_ path: String) {
    for item in items(load(path)) {
        let short = shortVersion(item)
        for (kind, element) in enclosures(item) {
            let signature = sparkleAttribute(element, "edSignature") ?? ""
            let url = element.attribute(forName: "url")?.stringValue ?? ""
            print([short, kind, fileName(element), signature, url].joined(separator: "\t"))
        }
    }
}

func rewrite(_ path: String, _ prefix: String, _ app: String) {
    let doc = load(path)
    let base = prefix.hasSuffix("/") ? String(prefix.dropLast()) : prefix
    for item in items(doc) {
        let short = shortVersion(item)
        for (kind, element) in enclosures(item) {
            guard let url = element.attribute(forName: "url") else {
                fail("an enclosure of \(short) has no URL")
            }
            let file = kind == "full" ? "\(app).dmg" : fileName(element)
            url.stringValue = "\(base)/v\(short)/\(file)"
        }
    }
    // The options generate_appcast writes with.
    let data = doc.xmlData(options: [.nodeCompactEmptyElement, .nodePrettyPrint])
    do {
        _ = try XMLDocument(data: data, options: [])
        try data.write(to: URL(fileURLWithPath: path), options: .atomic)
    } catch {
        fail("cannot write \(path): \(error.localizedDescription)")
    }
}

let args = Array(CommandLine.arguments.dropFirst())
switch (args.first, args.count) {
case ("public-key", 1): publicKey()
case ("items", 2): listItems(args[1])
case ("notes", 4): writeNotes(args[1], args[2], args[3])
case ("enclosures", 2): listEnclosures(args[1])
case ("rewrite", 4): rewrite(args[1], args[2], args[3])
default:
    fail(
        "usage: public-key | items <appcast> | notes <appcast> <dir> <app> | enclosures <appcast> | rewrite <appcast> <prefix> <app>"
    )
}
