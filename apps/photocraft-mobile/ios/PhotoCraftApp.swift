import SwiftUI
import UniformTypeIdentifiers

/// PhotoCraft for iPad. CraftKit provides the window: the canvas, Liquid Glass toolbar, layers
/// inspector, and the system document picker and exporter.
@main
struct PhotoCraftApp: App {
    var body: some Scene {
        WindowGroup {
            CraftShell(documentTypes: PhotoCraftApp.documentTypes)
        }
    }

    /// What PhotoCraft opens: its own documents, Photoshop documents, then any image.
    static let documentTypes: [UTType] = [UTType("dev.craft.photocraft.document"), UTType("com.adobe.photoshop-image"), .image].compactMap { $0 }
}
