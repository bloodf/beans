package main

import "github.com/egoist/mygo/ui"

// Native onboarding shell (adapted from Lorca onboarding.go). Create/restore/
// pair complete in the full Solid client; this surface establishes step
// navigation, accessible names, and localization for the native foundation.
type nativeOnboardingStep int

const (
	nativeOnboardingWelcome nativeOnboardingStep = iota
	nativeOnboardingCreate
	nativeOnboardingRestore
	nativeOnboardingPair
)

func (n *nativeDesktop) onboardingView(c *ui.Context) {
	p := nativeColors(c)
	if n.onboardingStep < nativeOnboardingWelcome || n.onboardingStep > nativeOnboardingPair {
		n.onboardingStep = nativeOnboardingWelcome
	}
	ui.Column(c).Grow(1).Padding(32).Gap(16).Justify(ui.Center).AlignItems(ui.Center).Children(func() {
		ui.Text(c, L("Welcome to Beans")).Bold().FontSize(24).TextColor(p.Label)
		switch n.onboardingStep {
		case nativeOnboardingCreate:
			ui.Text(c, L("Create a new identity"))
		case nativeOnboardingRestore:
			ui.Text(c, L("Restore from backup phrase"))
		case nativeOnboardingPair:
			ui.Text(c, L("Pair with another Device"))
		default:
			ui.Text(c, L("Create or restore an identity in the full client."))
		}
		ui.Row(c).Gap(8).Children(func() {
			if n.onboardingStep > nativeOnboardingWelcome {
				if pushButton(c, L("Back"), pushOptions{}).Clicked() {
					n.onboardingStep--
				}
			}
			if n.onboardingStep == nativeOnboardingWelcome {
				if pushButton(c, L("Create a new identity"), pushOptions{Kind: buttonPrimary}).Clicked() {
					n.onboardingStep = nativeOnboardingCreate
				}
				if pushButton(c, L("Restore from backup phrase"), pushOptions{}).Clicked() {
					n.onboardingStep = nativeOnboardingRestore
				}
				if pushButton(c, L("Pair with another Device"), pushOptions{}).Clicked() {
					n.onboardingStep = nativeOnboardingPair
				}
			} else if pushButton(c, L("Continue in full client…"), pushOptions{Kind: buttonPrimary}).Clicked() {
				app.showOnboarding()
			}
		})
		if pushButton(c, L("Open full client"), pushOptions{}).Clicked() {
			app.showMainWindow()
		}
	})
}
