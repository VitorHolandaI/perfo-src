import QtQuick
import Quickshell
import qs.Commons
import qs.Ui

Column {
  id: historyPage

  property var history: []
  property color foreground: Color.foreground
  property string fontFamily: Style.font.family
  property string metric: "CPU"
  property int scrubIndex: -1
  property bool isRecording: true
  property bool isPlaying: false
  property string zoomLabel: "2m"
  property string customSpanText: "40m"
  property int customSpanSeconds: 2400
  property string exportStatus: ""

  signal toggleRecordingRequested()
  signal requestCapacity(int samples)

  readonly property bool inputActiveFocus: spanInput.activeFocus
  readonly property bool isCustomZoom: zoomLabel === customSpanText

  readonly property int effectiveIndex: {
    if (history.length === 0) return -1
    if (scrubIndex < 0 || scrubIndex >= history.length) return history.length - 1
    return scrubIndex
  }

  readonly property var selectedSample: {
    if (history.length === 0 || effectiveIndex < 0) return null
    return history[effectiveIndex]
  }

  readonly property bool isLive: effectiveIndex === history.length - 1

  spacing: Style.space(5)

  Timer {
    id: playbackTimer
    interval: 1000
    repeat: true
    running: historyPage.isPlaying && historyPage.history.length > 0
    onTriggered: {
      var next = historyPage.effectiveIndex + 1
      if (next >= historyPage.history.length) {
        historyPage.scrubIndex = -1
        historyPage.isPlaying = false
      } else {
        historyPage.scrubIndex = next
      }
    }
  }

  Timer {
    id: exportStatusTimer
    interval: 4000
    repeat: false
    onTriggered: historyPage.exportStatus = ""
  }

  Process {
    id: exportProc
    property string targetFilename: ""
    onExited: function(exitCode, exitStatus) {
      if (exitCode === 0) {
        historyPage.exportStatus = "Saved: ~/" + targetFilename
      } else {
        historyPage.exportStatus = "Export failed"
      }
      exportStatusTimer.restart()
    }
  }

  // Row 1: Title, metric selector, zoom presets, and custom span input (e.g. 40m, 40h)
  Row {
    width: historyPage.width
    height: Style.space(22)
    spacing: Style.space(5)

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: "TIMELINE"
      color: historyPage.foreground
      opacity: 0.65
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: true
    }

    // Metric selector pills: CPU, MEM, IO, GPU
    Row {
      spacing: Style.space(3)
      anchors.verticalCenter: parent.verticalCenter

      Repeater {
        model: ["CPU", "MEM", "IO", "GPU"]
        delegate: Rectangle {
          width: Style.space(36)
          height: Style.space(18)
          radius: Style.cornerRadius
          color: historyPage.metric === modelData ? Color.accent : "transparent"
          border.color: historyPage.foreground
          border.width: 1
          opacity: historyPage.metric === modelData ? 1.0 : 0.6

          PlainText {
            anchors.centerIn: parent
            text: modelData
            color: historyPage.metric === modelData ? Color.surface : historyPage.foreground
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: historyPage.metric === modelData
          }

          MouseArea {
            anchors.fill: parent
            onClicked: historyPage.metric = modelData
          }
        }
      }
    }

    Item { width: Style.space(4); height: 1 }

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: "SPAN"
      color: historyPage.foreground
      opacity: 0.5
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
    }

    // Zoom/Span range presets: 2m, 15m, 1h, ALL
    Row {
      spacing: Style.space(3)
      anchors.verticalCenter: parent.verticalCenter

      Repeater {
        model: ["2m", "15m", "1h", "ALL"]
        delegate: Rectangle {
          width: Style.space(30)
          height: Style.space(18)
          radius: Style.cornerRadius
          color: historyPage.zoomLabel === modelData ? Color.accent : "transparent"
          border.color: historyPage.foreground
          border.width: 1
          opacity: historyPage.zoomLabel === modelData ? 1.0 : 0.55

          PlainText {
            anchors.centerIn: parent
            text: modelData
            color: historyPage.zoomLabel === modelData ? Color.surface : historyPage.foreground
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: historyPage.zoomLabel === modelData
          }

          MouseArea {
            anchors.fill: parent
            onClicked: historyPage.zoomLabel = modelData
          }
        }
      }
    }

    // Custom span input: user can enter 40m, 40h, 10h, 30s, etc.
    Rectangle {
      id: customSpanBox
      width: Style.space(46)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: historyPage.isCustomZoom ? Color.accent : "transparent"
      border.color: spanInput.activeFocus ? Color.accent : historyPage.foreground
      border.width: 1
      opacity: (historyPage.isCustomZoom || spanInput.activeFocus) ? 1.0 : 0.65

      TextInput {
        id: spanInput
        anchors.fill: parent
        anchors.leftMargin: 2
        anchors.rightMargin: 2
        text: historyPage.customSpanText
        color: historyPage.isCustomZoom ? Color.surface : historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: historyPage.isCustomZoom
        horizontalAlignment: TextInput.AlignHCenter
        verticalAlignment: TextInput.AlignVCenter
        selectByMouse: true
        clip: true

        Keys.onEscapePressed: function(event) {
          spanInput.focus = false
          event.accepted = true
        }

        onAccepted: {
          historyPage.applyCustomDuration(text)
          spanInput.focus = false
        }

        onEditingFinished: {
          historyPage.applyCustomDuration(text)
        }
      }

      MouseArea {
        anchors.fill: parent
        visible: !spanInput.activeFocus
        onClicked: {
          historyPage.applyCustomDuration(spanInput.text)
          spanInput.forceActiveFocus()
          spanInput.selectAll()
        }
      }
    }

    Item { width: Style.space(4); height: 1 }

    // Total recorded length indicator
    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: historyPage.formatDuration(historyPage.history.length)
      color: historyPage.foreground
      opacity: 0.6
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
    }
  }

  // Row 2: Playback & Action Controls (REC, PLAY, LIVE, step buttons)
  Row {
    width: historyPage.width
    height: Style.space(20)
    spacing: Style.space(5)

    // REC / FREEZE button
    Rectangle {
      width: Style.space(54)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: "transparent"
      border.color: historyPage.isRecording ? Color.error : historyPage.foreground
      border.width: 1

      Row {
        anchors.centerIn: parent
        spacing: 4
        Rectangle {
          width: 6
          height: 6
          radius: 3
          anchors.verticalCenter: parent.verticalCenter
          color: historyPage.isRecording ? Color.error : historyPage.foreground
          opacity: historyPage.isRecording ? 1.0 : 0.4
        }
        PlainText {
          anchors.verticalCenter: parent.verticalCenter
          text: historyPage.isRecording ? "REC" : "PAUSED"
          color: historyPage.foreground
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
        }
      }

      MouseArea {
        anchors.fill: parent
        onClicked: historyPage.toggleRecordingRequested()
      }
    }

    // PLAY / PAUSE button
    Rectangle {
      width: Style.space(48)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: historyPage.isPlaying ? Color.accent : "transparent"
      border.color: historyPage.foreground
      border.width: 1

      PlainText {
        anchors.centerIn: parent
        text: historyPage.isPlaying ? "PAUSE" : "PLAY"
        color: historyPage.isPlaying ? Color.surface : historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
      }

      MouseArea {
        anchors.fill: parent
        onClicked: {
          if (historyPage.isPlaying) {
            historyPage.isPlaying = false
          } else {
            if (historyPage.effectiveIndex >= historyPage.history.length - 1) {
              historyPage.scrubIndex = Math.max(0, historyPage.history.length - historyPage.currentZoomSeconds())
            }
            historyPage.isPlaying = true
          }
        }
      }
    }

    // Step -1s button
    Rectangle {
      width: Style.space(22)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: "transparent"
      border.color: historyPage.foreground
      border.width: 1

      PlainText {
        anchors.centerIn: parent
        text: "<"
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: true
      }

      MouseArea {
        anchors.fill: parent
        onClicked: {
          var target = historyPage.effectiveIndex - 1
          if (target >= 0) {
            historyPage.scrubIndex = target
            historyPage.isPlaying = false
          }
        }
      }
    }

    // Step +1s button
    Rectangle {
      width: Style.space(22)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: "transparent"
      border.color: historyPage.foreground
      border.width: 1

      PlainText {
        anchors.centerIn: parent
        text: ">"
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: true
      }

      MouseArea {
        anchors.fill: parent
        onClicked: {
          var target = historyPage.effectiveIndex + 1
          if (target < historyPage.history.length) {
            historyPage.scrubIndex = target
            historyPage.isPlaying = false
          }
        }
      }
    }

    // Jump to LIVE button
    Rectangle {
      width: Style.space(38)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: historyPage.isLive ? Color.accent : "transparent"
      border.color: historyPage.foreground
      border.width: 1
      opacity: historyPage.isLive ? 1.0 : 0.6

      PlainText {
        anchors.centerIn: parent
        text: "LIVE"
        color: historyPage.isLive ? Color.surface : historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: historyPage.isLive
      }

      MouseArea {
        anchors.fill: parent
        onClicked: {
          historyPage.scrubIndex = -1
          historyPage.isPlaying = false
        }
      }
    }

    // Export to TXT button
    Rectangle {
      width: Style.space(48)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: "transparent"
      border.color: historyPage.foreground
      border.width: 1
      opacity: exportProc.running ? 0.4 : 0.75

      PlainText {
        anchors.centerIn: parent
        text: "EXPORT"
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: true
      }

      MouseArea {
        anchors.fill: parent
        enabled: !exportProc.running
        onClicked: historyPage.exportReport()
      }
    }

    // Timing summary / export status text
    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: historyPage.exportStatus.length > 0 ? historyPage.exportStatus : historyPage.timingLabel()
      color: historyPage.exportStatus.length > 0 ? Color.accent : (historyPage.isLive ? historyPage.foreground : Color.accent)
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: historyPage.exportStatus.length > 0 || !historyPage.isLive
      elide: Text.ElideRight
      width: historyPage.width - Style.space(262)
    }
  }

  // Interactive timeline sparkline container
  Rectangle {
    id: timelineBox
    width: historyPage.width
    height: Style.space(44)
    color: "transparent"
    border.color: historyPage.foreground
    border.width: 1
    radius: Style.cornerRadius

    Row {
      id: barsRow
      anchors.fill: parent
      anchors.margins: 3
      spacing: 1

      Repeater {
        id: timelineRepeater
        model: historyPage.visibleBars()

        delegate: Rectangle {
          id: barDelegate
          readonly property bool isSelected: modelData.containsIndex(historyPage.effectiveIndex)
          readonly property real sampleValue: Number(modelData.value) || 0
          readonly property real maxMetricValue: historyPage.maxMetric(historyPage.metric)

          width: Math.max(1, (barsRow.width / Math.max(1, timelineRepeater.count)) - 1)
          height: Math.max(2, barsRow.height * Math.min(1.0, sampleValue / Math.max(1.0, maxMetricValue)))
          anchors.bottom: parent.bottom

          color: isSelected
            ? Color.accent
            : (modelData.rawIndex === historyPage.history.length - 1
                ? Qt.rgba(historyPage.foreground.r, historyPage.foreground.g, historyPage.foreground.b, 0.75)
                : Qt.rgba(historyPage.foreground.r, historyPage.foreground.g, historyPage.foreground.b, 0.35))

          Rectangle {
            visible: barDelegate.isSelected
            width: parent.width
            height: 3
            color: Color.accent
            anchors.bottom: parent.top
            anchors.bottomMargin: 1
          }

          MouseArea {
            anchors.fill: parent
            hoverEnabled: true
            onClicked: {
              historyPage.scrubIndex = modelData.rawIndex
              historyPage.isPlaying = false
            }
          }
        }
      }
    }

    PlainText {
      anchors.centerIn: parent
      visible: historyPage.history.length === 0
      text: "collecting history snapshots..."
      color: historyPage.foreground
      opacity: 0.5
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
    }
  }

  // Section title for process inspector
  Row {
    width: historyPage.width
    PlainText {
      text: "ACTIVE PROCESSES AT SELECTED TIMING"
      color: historyPage.foreground
      opacity: 0.65
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
    }
  }

  // Process table headers
  Row {
    width: historyPage.width
    spacing: Style.space(8)

    PlainText { width: Style.space(48); text: "PID"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
    PlainText { width: Style.space(120); text: "PROCESS"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
    PlainText { width: Style.space(56); text: historyPage.metric; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption; horizontalAlignment: Text.AlignRight }
    PlainText { width: Style.space(48); text: "RAM"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption; horizontalAlignment: Text.AlignRight }
    PlainText { width: parent.width - Style.space(296); text: "COMMAND"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
  }

  // Process rows at selected sample
  Repeater {
    model: historyPage.sortedProcesses()

    delegate: Row {
      width: historyPage.width
      height: Style.space(18)
      spacing: Style.space(8)

      PlainText {
        width: Style.space(48)
        text: String(modelData.pid)
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      PlainText {
        width: Style.space(120)
        text: historyPage.cleanName(modelData.name || modelData.cmd, modelData.pid)
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        font.bold: true
        elide: Text.ElideRight
      }

      PlainText {
        width: Style.space(56)
        text: historyPage.metricCellText(modelData)
        color: Color.accent
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }

      PlainText {
        width: Style.space(48)
        text: historyPage.formatBytes(modelData.mem_bytes)
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }

      PlainText {
        width: parent.width - Style.space(296)
        text: String(modelData.cmd || "")
        color: historyPage.foreground
        opacity: 0.7
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        elide: Text.ElideRight
      }
    }
  }

  PlainText {
    visible: historyPage.sortedProcesses().length === 0
    text: "no process activity recorded for this sample"
    color: historyPage.foreground
    opacity: 0.55
    font.family: historyPage.fontFamily
    font.pixelSize: Style.font.bodySmall
  }

  // Helper functions
  function currentZoomSeconds() {
    if (zoomLabel === "2m") return 120
    if (zoomLabel === "15m") return 900
    if (zoomLabel === "1h") return 3600
    if (zoomLabel === "ALL") return history.length
    return customSpanSeconds
  }

  function applyCustomDuration(input) {
    var secs = parseDuration(input)
    if (secs > 0) {
      customSpanSeconds = secs
      customSpanText = input.trim()
      zoomLabel = customSpanText
      historyPage.requestCapacity(secs)
    }
  }

  function parseDuration(input) {
    if (!input) return 120
    var str = String(input).trim().toLowerCase()
    var match = str.match(/^([0-9]+(?:\.[0-9]+)?)\s*([a-z]*)$/)
    if (!match) return 120
    var val = parseFloat(match[1])
    if (isNaN(val) || val <= 0) return 120
    var u = match[2]
    if (u.length > 0) {
      if (u[0] === "h") return Math.round(val * 3600)
      if (u[0] === "m") return Math.round(val * 60)
      if (u[0] === "d") return Math.round(val * 86400)
      if (u[0] === "s") return Math.round(val)
    }
    if (val <= 120) return Math.round(val * 60)
    return Math.round(val)
  }

  function visibleBars() {
    if (!history || history.length === 0) return []
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var sliceCount = history.length - startIdx
    var maxBars = 100

    if (sliceCount <= maxBars) {
      var bars = []
      for (var i = startIdx; i < history.length; i++) {
        var rawSample = history[i]
        bars.push({
          rawIndex: i,
          value: metricValue(rawSample),
          containsIndex: function(target) { return target === this.rawIndex }
        })
      }
      return bars
    }

    // Downsample into buckets for long time spans (such as 40m, 1h, 40h)
    var bucketSize = sliceCount / maxBars
    var downsampled = []
    for (var b = 0; b < maxBars; b++) {
      var bStart = Math.floor(startIdx + b * bucketSize)
      var bEnd = Math.min(history.length, Math.floor(startIdx + (b + 1) * bucketSize))
      if (bStart >= bEnd) continue

      var peakVal = 0
      var peakIdx = bStart
      for (var k = bStart; k < bEnd; k++) {
        var v = metricValue(history[k])
        if (v >= peakVal) {
          peakVal = v
          peakIdx = k
        }
      }
      downsampled.push({
        rawIndex: peakIdx,
        startIndex: bStart,
        endIndex: bEnd,
        value: peakVal,
        containsIndex: function(target) { return target >= this.startIndex && target < this.endIndex }
      })
    }
    return downsampled
  }

  function metricValue(sample) {
    if (!sample) return 0
    if (metric === "CPU") return Number(sample.cpu) || 0
    if (metric === "MEM") return Number(sample.mem) || 0
    if (metric === "IO") return Number(sample.io_mb) || 0
    if (metric === "GPU") return Number(sample.gpu) || 0
    return 0
  }

  function maxMetric(type) {
    if (type === "CPU" || type === "MEM" || type === "GPU") return 100.0
    var maxVal = 10.0
    for (var i = 0; i < history.length; i++) {
      var val = Number(history[i].io_mb) || 0
      if (val > maxVal) maxVal = val
    }
    return maxVal
  }

  function timingLabel() {
    if (!selectedSample) return "No history recorded yet"
    var offset = history.length - 1 - effectiveIndex
    var prefix = isLive ? "LIVE (NOW): " : ("-" + formatDuration(offset) + " (" + selectedSample.timestamp + "): ")
    return prefix + "CPU " + selectedSample.cpu + "% | MEM " + selectedSample.mem + "% | IO " + formatRate(selectedSample.read_bps + selectedSample.write_bps) + " | GPU " + selectedSample.gpu + "%"
  }

  function sortedProcesses() {
    if (!selectedSample || !selectedSample.processes) return []
    var list = selectedSample.processes.slice()
    if (metric === "MEM") {
      list.sort(function(a, b) { return (Number(b.mem_bytes) || 0) - (Number(a.mem_bytes) || 0) })
    } else {
      list.sort(function(a, b) { return (Number(b.cpu_percent) || 0) - (Number(a.cpu_percent) || 0) })
    }
    return list.slice(0, 5)
  }

  function metricCellText(proc) {
    if (metric === "MEM") {
      return formatBytes(proc.mem_bytes)
    }
    return Math.round(Number(proc.cpu_percent) || 0) + "%"
  }

  function cleanName(command, pid) {
    var executable = String(command || "").trim().split(/\s+/)[0]
    if (!executable) return String(pid)
    var slash = executable.lastIndexOf("/")
    if (slash >= 0) executable = executable.slice(slash + 1)
    executable = executable.replace(/^["']+|["']+$/g, "")
    return executable || String(pid)
  }

  function formatBytes(bytes) {
    var value = Number(bytes)
    if (!isFinite(value) || value <= 0) return "--"
    if (value >= 1073741824) return (value / 1073741824).toFixed(1) + "G"
    if (value >= 1048576) return (value / 1048576).toFixed(0) + "M"
    if (value >= 1024) return (value / 1024).toFixed(0) + "K"
    return Math.round(value) + "B"
  }

  function formatRate(bytes) {
    return formatBytes(bytes) + "/s"
  }

  function formatDuration(seconds) {
    var s = Math.max(0, Math.floor(Number(seconds) || 0))
    if (s < 60) return s + "s"
    if (s < 3600) {
      var rem = s % 60
      return Math.floor(s / 60) + "m" + (rem > 0 ? " " + rem + "s" : "")
    }
    var h = Math.floor(s / 3600)
    var m = Math.floor((s % 3600) / 60)
    return h + "h" + (m > 0 ? " " + m + "m" : "")
  }

  function exportReport() {
    if (!history || history.length === 0) {
      exportStatus = "No history to export"
      exportStatusTimer.restart()
      return
    }
    var now = new Date()
    var datePart = now.getFullYear() +
      ("0" + (now.getMonth() + 1)).slice(-2) +
      ("0" + now.getDate()).slice(-2)
    var timePart = ("0" + now.getHours()).slice(-2) +
      ("0" + now.getMinutes()).slice(-2) +
      ("0" + now.getSeconds()).slice(-2)
    var filename = "perfo-history-" + datePart + "-" + timePart + ".txt"
    var fullPath = "~/" + filename
    var report = generateExportText(now)

    exportProc.targetFilename = filename
    exportProc.command = [
      "python3",
      "-c",
      "import sys, pathlib; p = pathlib.Path(sys.argv[1]).expanduser(); p.write_text(sys.argv[2], encoding='utf-8')",
      fullPath,
      report
    ]
    exportProc.running = true
  }

  function padRight(str, len) {
    var s = String(str === undefined || str === null ? "" : str)
    while (s.length < len) s += " "
    return s
  }

  function padLeft(str, len) {
    var s = String(str === undefined || str === null ? "" : str)
    while (s.length < len) s = " " + s
    return s
  }

  function generateExportText(dateObj) {
    var lines = []
    var border = "================================================================================"
    var subBorder = "--------------------------------------------------------------------------------"

    lines.push(border)
    lines.push("                   PERFO - SYSTEM HISTORY & ANALYSIS REPORT")
    lines.push(border)
    lines.push("Generated at         : " + dateObj.toISOString().replace("T", " ").substr(0, 19))
    lines.push("Active Metric Focus  : " + historyPage.metric)
    lines.push("Timeline View Span   : " + historyPage.zoomLabel + " (" + historyPage.currentZoomSeconds() + "s)")
    lines.push("Total Recorded Time  : " + historyPage.formatDuration(historyPage.history.length) + " (" + historyPage.history.length + " samples in RAM)")
    lines.push("Current View State   : " + (historyPage.isLive ? "LIVE" : "SCRUBBED (Index: " + historyPage.effectiveIndex + ")"))
    lines.push("")

    var sample = historyPage.selectedSample
    lines.push(border)
    lines.push("                   1. SNAPSHOT AT SELECTED TIMING (" + (sample ? sample.timestamp : "--") + ")")
    lines.push(border)
    if (sample) {
      lines.push("Overall CPU Usage    : " + sample.cpu + "%")
      lines.push("Overall Memory Usage : " + sample.mem + "%")
      lines.push("Total Disk I/O Rate  : " + (Number(sample.io_mb) || 0).toFixed(1) + " MB/s (Read: " + historyPage.formatBytes(sample.read_bps) + "/s, Write: " + historyPage.formatBytes(sample.write_bps) + "/s)")
      lines.push("GPU Usage            : " + sample.gpu + "%")
      lines.push("")
      lines.push("Active Processes at this timing:")
      lines.push(padRight("PID", 8) + " | " + padLeft("% CPU", 7) + " | " + padLeft("RAM", 10) + " | " + padRight("PROCESS", 18) + " | COMMAND")
      lines.push(subBorder)
      var procs = historyPage.sortedProcesses()
      for (var i = 0; i < procs.length; i++) {
        var p = procs[i]
        var pName = historyPage.cleanName(p.name || p.cmd, p.pid)
        lines.push(
          padRight(p.pid, 8) + " | " +
          padLeft((Number(p.cpu_percent) || 0).toFixed(1) + "%", 7) + " | " +
          padLeft(historyPage.formatBytes(p.mem_bytes), 10) + " | " +
          padRight(pName, 18) + " | " +
          (p.cmd || pName)
        )
      }
    } else {
      lines.push("No sample data available.")
    }
    lines.push("")

    lines.push(border)
    lines.push("                   2. TIMELINE METRICS & PEAKS SUMMARY")
    lines.push(border)
    var peakCpu = 0, peakCpuTime = "--", peakCpuProc = "--"
    var peakMem = 0, peakMemTime = "--"
    var peakIo = 0, peakIoTime = "--"
    var peakGpu = 0, peakGpuTime = "--"
    var totalCpu = 0, totalMem = 0

    for (var k = 0; k < historyPage.history.length; k++) {
      var s = historyPage.history[k]
      totalCpu += Number(s.cpu) || 0
      totalMem += Number(s.mem) || 0

      if ((Number(s.cpu) || 0) >= peakCpu) {
        peakCpu = Number(s.cpu) || 0
        peakCpuTime = s.timestamp
        if (s.processes && s.processes.length > 0) {
          peakCpuProc = s.processes[0].name || s.processes[0].cmd || "--"
        }
      }
      if ((Number(s.mem) || 0) >= peakMem) {
        peakMem = Number(s.mem) || 0
        peakMemTime = s.timestamp
      }
      if ((Number(s.io_mb) || 0) >= peakIo) {
        peakIo = Number(s.io_mb) || 0
        peakIoTime = s.timestamp
      }
      if ((Number(s.gpu) || 0) >= peakGpu) {
        peakGpu = Number(s.gpu) || 0
        peakGpuTime = s.timestamp
      }
    }

    var avgCpu = historyPage.history.length > 0 ? (totalCpu / historyPage.history.length).toFixed(1) : "0"
    var avgMem = historyPage.history.length > 0 ? (totalMem / historyPage.history.length).toFixed(1) : "0"

    lines.push("Peak CPU Usage       : " + peakCpu + "% at " + peakCpuTime + " (Top: " + peakCpuProc + ")")
    lines.push("Peak Memory Usage    : " + peakMem + "% at " + peakMemTime)
    lines.push("Peak Disk I/O Rate   : " + peakIo.toFixed(1) + " MB/s at " + peakIoTime)
    lines.push("Peak GPU Usage       : " + peakGpu + "% at " + peakGpuTime)
    lines.push("Average CPU Usage    : " + avgCpu + "%")
    lines.push("Average Memory Usage : " + avgMem + "%")
    lines.push("")

    lines.push(border)
    lines.push("                   3. RECORDED TIME SERIES SAMPLES")
    lines.push(border)
    lines.push(padRight("TIMESTAMP", 12) + " | " + padLeft("CPU %", 7) + " | " + padLeft("MEM %", 7) + " | " + padLeft("IO MB/s", 10) + " | " + padLeft("GPU %", 7) + " | TOP PROCESS")
    lines.push(subBorder)
    var step = Math.max(1, Math.floor(historyPage.history.length / 100))
    for (var j = 0; j < historyPage.history.length; j += step) {
      var sm = historyPage.history[j]
      var topP = (sm.processes && sm.processes.length > 0) ? (sm.processes[0].name || sm.processes[0].cmd || "") : ""
      lines.push(
        padRight(sm.timestamp, 12) + " | " +
        padLeft(sm.cpu + "%", 7) + " | " +
        padLeft(sm.mem + "%", 7) + " | " +
        padLeft((Number(sm.io_mb) || 0).toFixed(1), 10) + " | " +
        padLeft(sm.gpu + "%", 7) + " | " +
        topP
      )
    }
    lines.push(border)
    lines.push("End of Perfo History Report")
    lines.push("")
    return lines.join("\n")
  }
}
