import QtQuick
import Quickshell
import qs.Commons
import qs.Ui

Panel {
  id: root
  moduleName: "vitor.perfo"
  ipcTarget: "vitor.perfo"
  manageIpc: false

  property var anchorItem: null
  property var hostWidget: null
  property var snapshot: null
  property int page: 0
  readonly property var pageNames: ["DASH", "CPU", "IO", "NET", "MEM", "DISKS", "FANS", "GPU"]
  readonly property var barIdentity: hostWidget || root
  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property int tableMountWidth: Style.space(220)
  readonly property int tableMetricWidth: Style.space(104)
  readonly property int metricLabelWidth: Style.space(72)
  readonly property int metricValueWidth: Style.space(110)

  function toggle() {
    if (root.opened) root.close()
    else root.open()
  }

  function open() {
    root.controller.show()
  }

  function close() {
    root.controller.hide()
  }

  function openTerminal() {
    if (!root.bar) return
    root.close()
    root.bar.run("omarchy-launch-floating-terminal-with-presentation perfo")
  }

  function movePage(delta) {
    root.page = (root.page + delta + root.pageNames.length) % root.pageNames.length
  }

  function formatBytes(bytes) {
    var value = Number(bytes)
    if (!isFinite(value)) return "--"
    if (value >= 1073741824) return (value / 1073741824).toFixed(1) + "G"
    if (value >= 1048576) return (value / 1048576).toFixed(0) + "M"
    if (value >= 1024) return (value / 1024).toFixed(0) + "K"
    return Math.round(value) + "B"
  }

  function formatRate(bytes) {
    return formatBytes(bytes) + "/s"
  }

  function percent(value) {
    var number = Number(value)
    return isFinite(number) ? Math.max(0, Math.min(100, number)) : 0
  }

  function historyMax(values, floor) {
    var maximum = Number(floor) || 1
    if (!values) return maximum
    for (var index = 0; index < values.length; index++) maximum = Math.max(maximum, Number(values[index]) || 0)
    return maximum
  }

  // Guarded so a zero total never yields Infinity/NaN in a label or a width.
  readonly property real memPercent: (root.snapshot && root.snapshot.total_mem_bytes > 0)
    ? root.percent(root.snapshot.used_mem_bytes * 100 / root.snapshot.total_mem_bytes)
    : 0
  readonly property bool hasMemPercent: root.snapshot !== null && root.snapshot !== undefined
    && root.snapshot.total_mem_bytes > 0

  // Recomputed once per snapshot rather than once per sparkline bar.
  readonly property var ioHistory: root.snapshot ? root.busiestIoHistory() : { read: [], write: [] }

  // io_history is keyed by every /proc/diskstats device, in arbitrary order,
  // including virtual ones. Graph the busiest real device instead of whichever
  // key happens to come first.
  function busiestIoHistory() {
    if (!root.snapshot || !root.snapshot.io_history) return { read: [], write: [] }
    var keys = Object.keys(root.snapshot.io_history)
    var bestKey = ""
    var bestTotal = -1
    for (var index = 0; index < keys.length; index++) {
      var key = keys[index]
      if (/^(zram|loop|ram|sr|fd|dm-)/.test(key)) continue
      var candidate = root.snapshot.io_history[key]
      if (!candidate) continue
      var total = 0
      for (var series = 0; series < 2; series++) {
        var samples = candidate[series] || []
        for (var sample = 0; sample < samples.length; sample++) total += Number(samples[sample]) || 0
      }
      if (total > bestTotal) {
        bestTotal = total
        bestKey = key
      }
    }
    if (!bestKey) return { read: [], write: [] }
    var history = root.snapshot.io_history[bestKey]
    return { read: history[0] || [], write: history[1] || [] }
  }

  function allDisks() {
    return root.snapshot && root.snapshot.disks ? root.snapshot.disks : []
  }

  function disks() {
    return root.allDisks().slice(0, 5)
  }

  function interfaces() {
    return root.snapshot && root.snapshot.net && root.snapshot.net.ifaces
      ? root.snapshot.net.ifaces.slice(0, 5) : []
  }

  function fans() {
    return root.snapshot && root.snapshot.fans && root.snapshot.fans.fans
      ? root.snapshot.fans.fans.slice(0, 6) : []
  }

  function gpus() {
    return root.snapshot && root.snapshot.gpu && root.snapshot.gpu.devices
      ? root.snapshot.gpu.devices : []
  }

  function processName(command, pid) {
    var executable = String(command || "").trim().split(/\s+/)[0]
    if (!executable) return String(pid)
    var slash = executable.lastIndexOf("/")
    if (slash >= 0) executable = executable.slice(slash + 1)
    executable = executable.replace(/^["']+|["']+$/g, "")
    return executable || String(pid)
  }

  function processSummary(processes) {
    if (!processes || processes.length === 0) return "none"
    var labels = []
    for (var index = 0; index < Math.min(4, processes.length); index++) {
      var process = processes[index]
      var name = root.processName(process.name || process.cmd, process.pid)
      labels.push(name + " " + Math.round(Number(process.cpu_percent) || 0) + "%")
    }
    return labels.join("   ")
  }

  function topProcesses(limit) {
    if (!root.snapshot || !root.snapshot.processes) return []
    return root.snapshot.processes.slice(0, limit)
  }

  function switchPage(direction) {
    movePage(direction)
  }

  KeyboardPanel {
    id: panel
    anchorItem: root.anchorItem
    owner: root.barIdentity
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(500))
    contentHeight: panel.fittedContentHeight(content.implicitHeight)

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (dx !== 0) root.switchPage(dx)
      }
      onCloseRequested: root.close()
      onTextKey: function(text) {
        if (text === "h" || text === "H") root.switchPage(-1)
        else if (text === "l" || text === "L") root.switchPage(1)
      }
    }

    Column {
      id: content
      width: parent.width
      spacing: Style.space(12)

      Row {
        width: parent.width
        height: Style.space(34)

        Column {
          width: parent.width - Style.space(88)
          anchors.verticalCenter: parent.verticalCenter
          PlainText { text: "PERFO"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.title; font.bold: true }
          PlainText { text: root.pageNames[root.page] + " FOCUS"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
        }

        PlainText {
          width: Style.space(30)
          text: "<"
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.title
          horizontalAlignment: Text.AlignHCenter
          verticalAlignment: Text.AlignVCenter
          MouseArea { anchors.fill: parent; onClicked: root.switchPage(-1) }
        }

        PlainText {
          width: Style.space(30)
          text: ">"
          color: root.foreground
          font.family: root.fontFamily
          font.pixelSize: Style.font.title
          horizontalAlignment: Text.AlignHCenter
          verticalAlignment: Text.AlignVCenter
          MouseArea { anchors.fill: parent; onClicked: root.switchPage(1) }
        }

        PlainText {
          width: Style.space(28)
          text: (root.page + 1) + "/" + root.pageNames.length
          color: root.foreground
          opacity: 0.65
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          horizontalAlignment: Text.AlignRight
          verticalAlignment: Text.AlignVCenter
        }
      }

      Item {
        width: parent.width
        height: Style.space(260)
        clip: true

        Column {
          id: dashboardPage
          anchors.fill: parent
          spacing: Style.space(8)
          visible: root.page === 0
          PlainText { text: "SYSTEM OVERVIEW"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
           Row {
             width: parent.width
             spacing: Style.space(16)
             PlainText { width: root.metricValueWidth; text: root.snapshot ? "CPU " + Math.round(root.snapshot.overall_percent) + "%" : "CPU --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricValueWidth; text: root.hasMemPercent ? "MEM " + Math.round(root.memPercent) + "%" : "MEM --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? "LOAD " + Number(root.snapshot.load_avg[0]).toFixed(2) : "LOAD --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
          }
          Rectangle {
            width: parent.width
            height: Style.space(42)
            color: "transparent"
            border.color: root.foreground
            border.width: 1
            Row {
              anchors.fill: parent
              anchors.margins: 4
              spacing: 1
              Repeater {
                id: dashboardHistoryRepeater
                model: root.snapshot ? root.snapshot.cpu_history : []
                delegate: Rectangle { width: Math.max(1, (parent.width / Math.max(1, dashboardHistoryRepeater.count)) - 1); height: Math.max(2, parent.height * root.percent(modelData) / 100); anchors.bottom: parent.bottom; color: Color.accent }
              }
            }
          }
           Row {
             width: parent.width
             spacing: Style.space(12)
             PlainText { width: root.metricValueWidth * 2; text: root.snapshot ? "NET " + root.formatRate(root.snapshot.net.totals.rx_bps) + " / " + root.formatRate(root.snapshot.net.totals.tx_bps) : "NET --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             PlainText { width: root.metricValueWidth * 2; text: root.snapshot ? "IO " + root.formatRate(root.totalRead()) + " / " + root.formatRate(root.totalWrite()) : "IO --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
          }
           PlainText { text: root.snapshot && root.disks().length > 0 ? "DISK " + root.disks()[0].mount + " " + Number(root.disks()[0].percent).toFixed(0) + "%" : "DISK --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
           PlainText { text: root.snapshot ? "FANS " + root.fans().length : "FANS --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
           PlainText { text: root.gpus().length > 0 ? "GPU " + root.gpus()[0].vendor + " " + (root.gpus()[0].usage_percent === null ? "--" : Number(root.gpus()[0].usage_percent).toFixed(0) + "%") : "GPU --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
           PlainText { text: "TOP PROCESSES"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
           ProcessGrid {
             id: dashboardProcessGrid
             width: parent.width
             processes: root.topProcesses(4)
             foreground: root.foreground
             fontFamily: root.fontFamily
             columnSpacing: Style.space(16)
             rowSpacing: Style.space(3)
           }
        }

        Column {
          id: cpuPage
          anchors.fill: parent
          spacing: Style.space(8)
          visible: root.page === 1
          PlainText { text: "CPU HISTORY"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
          Rectangle {
            width: parent.width
            height: Style.space(72)
            color: "transparent"
            border.color: root.foreground
            border.width: 1
            Row {
              anchors.fill: parent
              anchors.margins: 4
              spacing: 1
              Repeater {
                id: cpuHistoryRepeater
                model: root.snapshot ? root.snapshot.cpu_history : []
                delegate: Rectangle { width: Math.max(1, (parent.width / Math.max(1, cpuHistoryRepeater.count)) - 1); height: Math.max(2, parent.height * root.percent(modelData) / 100); anchors.bottom: parent.bottom; color: Color.accent }
              }
            }
          }
          PlainText { text: "PER-CORE"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
          Row {
            width: parent.width
            height: Style.space(28)
            spacing: 2
            Repeater {
              id: coreRepeater
              model: root.snapshot ? root.snapshot.per_core : []
              delegate: Rectangle { width: Math.max(2, (parent.width / Math.max(1, coreRepeater.count)) - 2); height: Math.max(2, parent.height * root.percent(modelData) / 100); anchors.bottom: parent.bottom; color: Color.accent }
            }
          }
           Row {
             width: parent.width
             spacing: Style.space(16)
             PlainText { width: root.metricValueWidth; text: root.snapshot ? "LOAD " + Number(root.snapshot.load_avg[0]).toFixed(2) : "LOAD --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? "IOWAIT " + Number(root.snapshot.iowait_percent).toFixed(1) + "%" : "IOWAIT --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
             PlainText { width: root.metricValueWidth; text: root.snapshot && root.snapshot.cpu_temp_c !== null ? "TEMP " + Number(root.snapshot.cpu_temp_c).toFixed(0) + "C" : "TEMP --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
          }
           PlainText { text: "TOP PROCESSES"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
           ProcessGrid {
             width: parent.width
             processes: root.topProcesses(8)
             foreground: root.foreground
             fontFamily: root.fontFamily
             columnSpacing: Style.space(16)
             rowSpacing: Style.space(3)
           }
        }

        Column {
          id: ioPage
          anchors.fill: parent
          spacing: Style.space(8)
          visible: root.page === 2
          PlainText { text: "DISK ACTIVITY"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
           Row {
             width: parent.width
             PlainText { width: root.metricLabelWidth; text: "READ"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? root.formatRate(root.totalRead()) : "--"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricLabelWidth; text: "WRITE"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? root.formatRate(root.totalWrite()) : "--"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
           }
           Row {
             width: parent.width
             PlainText { width: root.metricLabelWidth; text: "PRESSURE"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? Number(root.snapshot.io_pressure_some[0]).toFixed(2) : "--"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             PlainText { width: Style.space(18); text: "/"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? Number(root.snapshot.io_pressure_some[1]).toFixed(2) : "--"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             PlainText { width: Style.space(18); text: "/"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? Number(root.snapshot.io_pressure_some[2]).toFixed(2) : "--"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
           }
          Rectangle {
            width: parent.width
            height: Style.space(56)
            color: "transparent"
            border.color: root.foreground
            border.width: 1
            Row {
              anchors.fill: parent
              anchors.margins: 4
              spacing: 1
              Repeater {
                id: ioHistoryRepeater
                model: root.ioHistory.read
                delegate: Rectangle { width: Math.max(1, (parent.width / Math.max(1, ioHistoryRepeater.count)) - 1); height: Math.max(2, parent.height * Number(modelData) / root.historyMax(root.ioHistory.read, 1)); anchors.bottom: parent.bottom; color: Color.accent }
              }
            }
          }
           Repeater {
             model: root.disks()
             delegate: Row {
               width: ioPage.width
               PlainText { width: root.tableMountWidth; text: modelData.mount; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight }
               PlainText { width: root.tableMetricWidth; text: "R " + root.formatRate(modelData.read_bps); color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
               PlainText { width: root.tableMetricWidth; text: "W " + root.formatRate(modelData.write_bps); color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             }
          }
        }

        Column {
          id: netPage
          anchors.fill: parent
          spacing: Style.space(8)
          visible: root.page === 3
          PlainText { text: "NETWORK HISTORY"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
           Row {
             width: parent.width
             spacing: Style.space(14)
             PlainText { width: root.metricLabelWidth; text: "RX"; color: Color.accent; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; font.bold: true }
             PlainText { width: root.metricLabelWidth; text: "TX"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; font.bold: true }
          }
          Rectangle {
            width: parent.width
            height: Style.space(72)
            color: "transparent"
            border.color: root.foreground
            border.width: 1
            Column {
              anchors.fill: parent
              anchors.margins: 4
              spacing: 2
              Row {
                width: parent.width
                height: (parent.height - 2) / 2
                spacing: 1
                Repeater {
                  id: rxHistoryRepeater
                  model: root.snapshot ? root.snapshot.net.rx_history : []
                  delegate: Rectangle { width: Math.max(1, (parent.width / Math.max(1, rxHistoryRepeater.count)) - 1); height: Math.max(2, parent.height * Number(modelData) / root.historyMax(root.snapshot.net.rx_history, 1)); anchors.bottom: parent.bottom; color: Color.accent }
                }
              }
              Row {
                width: parent.width
                height: (parent.height - 2) / 2
                spacing: 1
                Repeater {
                  id: txHistoryRepeater
                  model: root.snapshot ? root.snapshot.net.tx_history : []
                  delegate: Rectangle { width: Math.max(1, (parent.width / Math.max(1, txHistoryRepeater.count)) - 1); height: Math.max(2, parent.height * Number(modelData) / root.historyMax(root.snapshot.net.tx_history, 1)); anchors.bottom: parent.bottom; color: root.foreground }
                }
              }
            }
          }
           Row {
             width: parent.width
             PlainText { width: root.metricLabelWidth; text: "RX"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? root.formatRate(root.snapshot.net.totals.rx_bps) : "--"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricLabelWidth; text: "TX"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? root.formatRate(root.snapshot.net.totals.tx_bps) : "--"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
           }
           Repeater {
             model: root.interfaces()
             delegate: Row {
               width: netPage.width
               PlainText { width: root.tableMountWidth; text: modelData.name; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight }
               PlainText { width: root.tableMetricWidth; text: "RX " + root.formatRate(modelData.rx_bps); color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
               PlainText { width: root.tableMetricWidth; text: "TX " + root.formatRate(modelData.tx_bps); color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
             }
           }
        }

        Column {
          id: memPage
          anchors.fill: parent
          spacing: Style.space(10)
          visible: root.page === 4
          PlainText { text: "MEMORY"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
          PlainText { text: root.hasMemPercent ? Math.round(root.memPercent) + "% USED" : "-- USED"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.display; font.bold: true }
          Rectangle { width: parent.width; height: Style.space(14); color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15); Rectangle { width: parent.width * root.memPercent / 100; height: parent.height; color: Color.accent } }
           Row {
             width: parent.width
             PlainText { width: root.metricLabelWidth; text: "USED"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? root.formatBytes(root.snapshot.used_mem_bytes) : "--"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
             PlainText { width: root.metricLabelWidth + Style.space(22); text: "AVAILABLE"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
             PlainText { width: root.metricValueWidth; text: root.snapshot ? root.formatBytes(root.snapshot.mem.available) : "--"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.body }
           }
          PlainText { text: root.snapshot ? "SWAP " + root.formatBytes(root.snapshot.mem.swap_used) + " / " + root.formatBytes(root.snapshot.mem.swap_total) : "SWAP --"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
          PlainText { text: root.snapshot ? "PRESSURE " + Number(root.snapshot.mem.psi_some_10).toFixed(2) + " / " + Number(root.snapshot.mem.psi_some_60).toFixed(2) + " / " + Number(root.snapshot.mem.psi_some_300).toFixed(2) : "PRESSURE --"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
        }

        Column {
          id: diskPage
          anchors.fill: parent
          spacing: Style.space(8)
          visible: root.page === 5
          PlainText { text: "FILESYSTEMS"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
          Repeater {
            model: root.disks()
            delegate: Item {
              width: diskPage.width
              height: Style.space(34)
              PlainText { width: Style.space(92); text: modelData.mount; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; anchors.verticalCenter: parent.verticalCenter; elide: Text.ElideRight }
              Rectangle { x: Style.space(96); y: Style.space(7); width: parent.width - Style.space(150); height: Style.space(10); color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.15); Rectangle { width: parent.width * root.percent(modelData.percent) / 100; height: parent.height; color: modelData.percent >= 90 ? Color.urgent : Color.accent } }
              PlainText { anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter; text: Number(modelData.percent).toFixed(0) + "%"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
            }
          }
        }

        Column {
          id: fanPage
          anchors.fill: parent
          spacing: Style.space(8)
          visible: root.page === 6
          PlainText { text: "COOLING / FANS"; color: root.foreground; opacity: 0.65; font.family: root.fontFamily; font.pixelSize: Style.font.caption }
          PlainText { text: root.snapshot && root.snapshot.cpu_temp_c !== null ? "CPU " + Number(root.snapshot.cpu_temp_c).toFixed(0) + "C" : "CPU TEMP --"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.subtitle; font.bold: true }
          PlainText { text: root.fans().length > 0 ? "EVERY DETECTED COOLER" : "NO READABLE COOLERS"; color: root.foreground; opacity: 0.7; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall }
          Repeater {
            model: root.fans()
            delegate: Row {
              width: fanPage.width
              height: Style.space(22)
              PlainText { width: root.tableMountWidth; text: modelData.label; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight; anchors.verticalCenter: parent.verticalCenter }
              PlainText { width: root.tableMetricWidth; text: modelData.rpm + " RPM"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; anchors.verticalCenter: parent.verticalCenter }
              PlainText { width: root.tableMetricWidth; text: "[" + modelData.chip + "]"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; elide: Text.ElideRight; anchors.verticalCenter: parent.verticalCenter }
            }
          }
        }

        Column {
          anchors.fill: parent
          visible: root.page === 7
          GpuPage {
            width: parent.width
            devices: root.gpus()
            processes: root.snapshot && root.snapshot.processes ? root.snapshot.processes : []
            totalMemoryBytes: root.snapshot ? root.snapshot.total_mem_bytes : 0
            foreground: root.foreground
            fontFamily: root.fontFamily
          }
        }
      }

      Rectangle {
        width: parent.width
        height: Style.space(38)
        color: "transparent"
        border.color: root.foreground
        border.width: 1
        radius: Style.cornerRadius
        PlainText { anchors.fill: parent; text: "OPEN FULL TUI"; color: root.foreground; font.family: root.fontFamily; font.pixelSize: Style.font.bodySmall; font.bold: true; horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter }
        MouseArea { anchors.fill: parent; onClicked: root.openTerminal() }
      }
    }
  }

  // read_bps/write_bps are per block device, while `disks` is per mount, so
  // several mounts on one device (btrfs subvolumes, bind mounts) repeat the
  // same rate. Count each device once, and use the untruncated list so a
  // filesystem past the fifth still contributes.
  function totalDeviceRate(field) {
    var total = 0
    var seen = ({})
    var list = root.allDisks()
    for (var index = 0; index < list.length; index++) {
      var entry = list[index]
      var device = String(entry.name || entry.mount || index)
      if (seen[device]) continue
      seen[device] = true
      total += Number(entry[field]) || 0
    }
    return total
  }

  function totalRead() {
    return root.totalDeviceRate("read_bps")
  }

  function totalWrite() {
    return root.totalDeviceRate("write_bps")
  }
}