import AppKit
import XCTest
@testable import Beans

@MainActor
final class MainMenuLifecycleTests: XCTestCase {
    func testRebuildWhilePreviousMenuRemainsAttachedUsesIndependentSubmenus() {
        let app = NSApplication.shared
        let savedMain = app.mainMenu
        let savedServices = app.servicesMenu
        let savedWindows = app.windowsMenu
        let savedHelp = app.helpMenu
        defer {
            app.mainMenu = savedMain
            app.servicesMenu = savedServices
            app.windowsMenu = savedWindows
            app.helpMenu = savedHelp
        }

        func descendants(of menu: NSMenu) -> [NSMenu] {
            [menu] + menu.items.compactMap(\.submenu).flatMap { descendants(of: $0) }
        }

        let previous = MainMenu.build()
        app.mainMenu = previous
        let previousMenus = descendants(of: previous)
        let previousOwners = previousMenus.map { $0.supermenu }

        // Language changes evaluate the new builder before replacing the installed menu.
        let replacement = MainMenu.build()
        let replacementMenus = descendants(of: replacement)
        let previousIDs = Set(previousMenus.map(ObjectIdentifier.init))
        XCTAssertTrue(previousIDs.isDisjoint(with: replacementMenus.map(ObjectIdentifier.init)),
                      "A submenu cannot belong to both menu bars during a language rebuild.")
        for (menu, owner) in zip(previousMenus, previousOwners) {
            XCTAssertTrue(menu.supermenu === owner, "Building a replacement must not reparent an old submenu.")
        }
        app.mainMenu = replacement
        XCTAssertTrue(app.mainMenu === replacement)
    }
}
