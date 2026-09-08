import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

BarWidget {
  id: barWidgetRoot
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

  readonly property bool isRecording: panelLoader.item ? panelLoader.item.isSessionRecording : false

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
  readonly property string fullText: (isRecording ? "● REC  " : "") + label
  readonly property bool opened: panelLoader.item ? panelLoader.item.opened === true : false
  readonly property bool popoutSwitchClosing: panelLoader.item ? panelLoader.item.popoutSwitchClosing === true : false

  implicitWidth: barWidgetRoot.vertical ? barWidgetRoot.barSize : button.implicitWidth
  implicitHeight: barWidgetRoot.barSize

  function injectPanel() {
    var target = panelLoader.item
    if (!target) return
    if ("bar" in target) target.bar = barWidgetRoot.bar
    if ("settings" in target) target.settings = barWidgetRoot.settings
    if ("anchorItem" in target) target.anchorItem = button
    if ("hostWidget" in target) target.hostWidget = barWidgetRoot
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
    if (barWidgetRoot.opened) barWidgetRoot.close()
    else barWidgetRoot.open()
  }

  onBarChanged: injectPanel()

  Process {
    id: collector
    command: [barWidgetRoot.binaryPath, "stream", "--json"]
    running: true
    stdout: SplitParser {
      onRead: function(line) {
        try {
          barWidgetRoot.snapshot = JSON.parse(line)
        } catch (error) {
          console.warn("vitor.perfo: invalid JSON snapshot", error)
        }
      }
    }
  }

  IpcHandler {
    target: "vitor.perfo"
    function open() { barWidgetRoot.open() }
    function close() { barWidgetRoot.close() }
    function show() { barWidgetRoot.open() }
    function hide() { barWidgetRoot.close() }
    function toggle() { barWidgetRoot.toggle() }
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
    function loadSession(p: string, id: string) { if (panelLoader.item && panelLoader.item.historyPageComp) panelLoader.item.historyPageComp.loadSession(p, id, false, false) }
  }

  Loader {
    id: panelLoader
    active: true
    source: Qt.resolvedUrl("Panel.qml")
    visible: false
    onLoaded: {
      barWidgetRoot.injectPanel()
      Qt.callLater(barWidgetRoot.injectPanel)
    }
  }

  WidgetButton {
    id: button
    anchors.fill: parent
    bar: barWidgetRoot.bar
    text: barWidgetRoot.fullText
    labelVisible: false
    horizontalMargin: 6
    tooltipText: barWidgetRoot.isRecording ? "Session recording active - Left: metrics | Middle: timeline | Right: full TUI" : "Left: metrics | Middle: timeline & play | Right: full TUI"

    Row {
      anchors.centerIn: parent
      spacing: Style.space(4)
      enabled: false

      // Pulsing red indicator when session is actively recording
      Row {
        visible: barWidgetRoot.isRecording
        spacing: Style.space(3)
        anchors.verticalCenter: parent.verticalCenter

        Rectangle {
          width: Style.space(6)
          height: Style.space(6)
          radius: Style.space(3)
          color: Color.urgent
          anchors.verticalCenter: parent.verticalCenter

          SequentialAnimation on opacity {
            running: barWidgetRoot.isRecording
            loops: Animation.Infinite
            NumberAnimation { to: 0.25; duration: 600; easing.type: Easing.InOutQuad }
            NumberAnimation { to: 1.0; duration: 600; easing.type: Easing.InOutQuad }
          }
        }

        PlainText {
          text: "REC"
          color: Color.urgent
          font.family: button.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
          anchors.verticalCenter: parent.verticalCenter
        }
      }

      PlainText {
        text: barWidgetRoot.label
        color: button.foreground
        font.family: button.fontFamily
        font.pixelSize: button.fontSize
        anchors.verticalCenter: parent.verticalCenter
      }
    }

    onPressed: function(b) {
      if (b === Qt.LeftButton) barWidgetRoot.toggle()
      else if (b === Qt.MiddleButton && panelLoader.item) {
        panelLoader.item.page = 8
        barWidgetRoot.open()
      }
      else if (b === Qt.RightButton && panelLoader.item) panelLoader.item.openTerminal()
    }
  }
}