// Draws the app icon: swift scripts/make-icon.swift <icon.png> [<layers-dir>]
// icon.png is the flat 1024px icon on the standard macOS grid, for the .icns that macOS 14
// and 15 use. With a layers directory it also writes the curve and the node as separate
// full-bleed layers, which the Icon Composer icon in bundle/AppIcon.icon is made of, so
// macOS 26 and later can render them as glass in every icon style.
// scripts/make-icon.sh runs this and assembles both icons.
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

let size: CGFloat = 1024
let space = CGColorSpace(name: CGColorSpace.sRGB)!

func color(_ hex: UInt32) -> CGColor {
    CGColor(
        colorSpace: space,
        components: [
            CGFloat((hex >> 16) & 0xff) / 255, CGFloat((hex >> 8) & 0xff) / 255,
            CGFloat(hex & 0xff) / 255, 1,
        ])!
}

func canvas() -> CGContext {
    CGContext(
        data: nil, width: Int(size), height: Int(size), bitsPerComponent: 8, bytesPerRow: 0,
        space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
}

func write(_ ctx: CGContext, to path: String) {
    let url = URL(fileURLWithPath: path)
    let destination = CGImageDestinationCreateWithURL(
        url as CFURL, UTType.png.identifier as CFString, 1, nil)!
    CGImageDestinationAddImage(destination, ctx.makeImage()!, nil)
    guard CGImageDestinationFinalize(destination) else { fatalError("could not write \(path)") }
}

/// The artwork laid out on `tile`, the icon's rounded body. Measurements are for the
/// 824pt body of the standard grid and scale with the tile.
struct Artwork {
    let tile: CGRect
    var s: CGFloat { tile.width / 824 }
    var left: CGFloat { tile.minX + 120 * s }
    var right: CGFloat { tile.maxX - 120 * s }
    var peakX: CGFloat { left + (right - left) * 0.34 }

    // One response curve: a boost on the left, a gentler cut on the right.
    func curve(_ x: CGFloat) -> CGFloat {
        let mid = tile.midY - 65 * s
        let dipX = left + (right - left) * 0.74
        let boost = 210 * s * exp(-pow((x - peakX) / (95 * s), 2))
        let cut = 120 * s * exp(-pow((x - dipX) / (85 * s), 2))
        return mid + boost - cut
    }

    var node: CGPoint { CGPoint(x: peakX, y: curve(peakX)) }

    func strokeCurve(_ ctx: CGContext) {
        let line = CGMutablePath()
        line.move(to: CGPoint(x: left, y: curve(left)))
        for x in stride(from: left, through: right, by: 2) {
            line.addLine(to: CGPoint(x: x, y: curve(x)))
        }
        ctx.addPath(line)
        ctx.setStrokeColor(color(0xf2f3f5))
        ctx.setLineWidth(40 * s)
        ctx.setLineCap(.round)
        ctx.setLineJoin(.round)
        ctx.strokePath()
    }

    func circle(_ radius: CGFloat) -> CGRect {
        CGRect(x: node.x - radius * s, y: node.y - radius * s, width: 2 * radius * s, height: 2 * radius * s)
    }

    /// The draggable node sitting on the boost.
    func fillNode(_ ctx: CGContext) {
        ctx.setFillColor(color(0x4da3ff))
        ctx.fillEllipse(in: circle(44))
    }
}

// The flat icon: rounded tile, gradient, curve, and the node in a dark ring.
let flat = canvas()
let art = Artwork(tile: CGRect(x: 100, y: 100, width: 824, height: 824))
let body = CGPath(roundedRect: art.tile, cornerWidth: 186, cornerHeight: 186, transform: nil)
flat.saveGState()
flat.addPath(body)
flat.clip()
let gradient = CGGradient(
    colorsSpace: space, colors: [color(0x2c2e35), color(0x15161a)] as CFArray,
    locations: [0, 1])!
flat.drawLinearGradient(
    gradient, start: CGPoint(x: 0, y: art.tile.maxY), end: CGPoint(x: 0, y: art.tile.minY),
    options: [])
art.strokeCurve(flat)
flat.setFillColor(color(0x1f2025))
flat.fillEllipse(in: art.circle(62))
art.fillNode(flat)
flat.restoreGState()
write(flat, to: CommandLine.arguments[1])

// The layers fill the whole canvas, which the system masks to its own icon shape. The
// background comes from the icon's fill, so the layers are transparent around the artwork.
if CommandLine.arguments.count > 2 {
    let dir = CommandLine.arguments[2]
    let full = Artwork(tile: CGRect(x: 0, y: 0, width: size, height: size))

    let curve = canvas()
    full.strokeCurve(curve)
    // A gap around the node instead of the dark ring, so it reads on any backdrop.
    curve.setBlendMode(.clear)
    curve.fillEllipse(in: full.circle(62))
    write(curve, to: "\(dir)/curve.png")

    let node = canvas()
    full.fillNode(node)
    write(node, to: "\(dir)/node.png")
}
