import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: root
  moduleName: "vitor.perfo"
  property var manifest: null
  property var snapshot: null

  // The shell injects `manifest` for bar, service and panel kinds only, never
  // for bar-widget, so the binary is resolved relative to this file instead.
  readonly property string bundledBinaryPath: {
    var resolved = String(Qt.resolvedUrl("bin/perfo"))
    return resolved.indexOf("file://") === 0 ? resolved.substring(7) : resolved
  }
  readonly property string binaryPath: {
    var override = Quickshell.env("PERFO_BIN")
    if (override) return override
    if (bundledBinaryPath) return bundledBinaryPath
    return Quickshell.env("HOME") + "/.local/bin/perfo"
  }

  readonly property string cpuLabel: snapshot ? "C " + Math.round(snapshot.overall_percent) + "%" : "C --"
  readonly property string memLabel: snapshot && snapshot.total_mem_bytes > 0
    ? "M " + Math.round(snapshot.used_mem_bytes * 100 / snapshot.total_mem_bytes) + "%"
    : "M --"
  readonly property string gpuLabel: snapshot && snapshot.gpu && snapshot.gpu.devices && snapshot.gpu.devices.length > 0 && snapshot.gpu.devices[0].usage_percent !== null
    ? "G " + Math.round(snapshot.gpu.devices[0].usage_percent) + "%"
    : ""
  readonly property string label: {
    var parts = [cpuLabel, memLabel]
    if (gpuLabel) parts.push(gpuLabel)
    return parts.join("  ")
  }
  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  readonly property bool popoutSwitchClosing: panelLoader.item ? panelLoader.item.popoutSwitchClosing === true : false

  implicitWidth: root.vertical ? root.barSize : button.implicitWidth
  implicitHeight: root.barSize

  function injectPanel() {
    var target = panelLoader.item
    if (!target) return
    if ("bar" in target) target.bar = root.bar
    if ("settings" in target) target.settings = root.settings
    if ("anchorItem" in target) target.anchorItem = button
    if ("hostWidget" in target) target.hostWidget = root
  }

  function open() {
    if (panelLoader.item) panelLoader.item.open()
  }

  function close() {
    if (panelLoader.item) panelLoader.item.close()
  }

  function closeForPopoutSwitch() {
    if (panelLoader.item && panelLoader.item.closeForPopoutSwitch) panelLoader.item.closeForPopoutSwitch()
  }

  function toggle() {
    if (root.opened) root.close()
    else root.open()
  }

  onBarChanged: injectPanel()

  Process {
    id: collector
    command: [root.binaryPath, "stream", "--json"]
    running: true
    stdout: SplitParser {
      onRead: function(line) {
        try {
          root.snapshot = JSON.parse(line)
        } catch (error) {
          console.warn("vitor.perfo: invalid JSON snapshot", error)
        }
      }
    }
  }

  IpcHandler {
    target: "vitor.perfo"
    function open() { root.open() }
    function close() { root.close() }
    function show() { root.open() }
    function hide() { root.close() }
    function toggle() { root.toggle() }
    function setPage(p: int) { if (panelLoader.item) panelLoader.item.page = p }
    function page(): int { return panelLoader.item ? panelLoader.item.page : 0 }
    function toggleRecording() { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.toggleSessionRecording() }
    function isRecording(): bool { return (panelLoader.item && panelLoader.item.historyPageComp) ? panelLoader.item.historyPageComp.isSessionRecording : false }
    function toggleSessionsMenu() { if (panelLoader.item) panelLoader.item.toggleSessionsMenu() }
    function setMetric(m: string) { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.metric = m }
    function setZoom(z: string) { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.zoomLabel = z }
    function openCustomInput() { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.customInputOpen = true }
    function applyCustomMinutes(m: int) { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.applyCustomMinutes(String(m)) }
    function jumpToLive() { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.jumpToLive() }
    function exportReport() { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.exportReport() }
  }

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: {
      root.injectPanel()
      Qt.callLater(root.injectPanel)
    }
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    text: root.label
    horizontalMargin: 6
    tooltipText: "Left: metrics | Middle: timeline & play | Right: full TUI"

    onPressed: function(b) {
      if (b === Qt.LeftButton) root.toggle()
      else if (b === Qt.MiddleButton && panelLoader.item) {
        panelLoader.item.page = 8
        root.open()
      }
      else if (b === Qt.RightButton && panelLoader.item) panelLoader.item.openTerminal()
    }
  }
}