import QtQuick
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

  signal toggleRecordingRequested()

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

  spacing: Style.space(6)

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

  // Top header: mode badges and action buttons
  Row {
    width: historyPage.width
    height: Style.space(24)
    spacing: Style.space(6)

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: "TIMELINE & REPLAY"
      color: historyPage.foreground
      opacity: 0.65
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
    }

    Item { width: Style.space(12); height: 1 }

    // Metric selector pills: CPU, MEM, IO, GPU
    Row {
      spacing: Style.space(4)
      anchors.verticalCenter: parent.verticalCenter

      Repeater {
        model: ["CPU", "MEM", "IO", "GPU"]
        delegate: Rectangle {
          width: Style.space(40)
          height: Style.space(20)
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

    Item { width: Style.space(8); height: 1 }

    // REC / FREEZE button
    Rectangle {
      width: Style.space(56)
      height: Style.space(20)
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
      width: Style.space(52)
      height: Style.space(20)
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
              historyPage.scrubIndex = 0
            }
            historyPage.isPlaying = true
          }
        }
      }
    }

    // Jump to LIVE button
    Rectangle {
      width: Style.space(42)
      height: Style.space(20)
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
  }

  // Interactive timeline sparkline container
  Rectangle {
    id: timelineBox
    width: historyPage.width
    height: Style.space(46)
    color: "transparent"
    border.color: historyPage.foreground
    border.width: 1
    radius: Style.cornerRadius

    Row {
      id: barsRow
      anchors.fill: parent
      anchors.margins: 4
      spacing: 1

      Repeater {
        id: timelineRepeater
        model: historyPage.history

        delegate: Rectangle {
          id: barDelegate
          readonly property bool isSelected: index === historyPage.effectiveIndex
          readonly property real sampleValue: historyPage.metricValue(modelData)
          readonly property real maxMetricValue: historyPage.maxMetric(historyPage.metric)

          width: Math.max(2, (barsRow.width / Math.max(1, timelineRepeater.count)) - 1)
          height: Math.max(3, barsRow.height * Math.min(1.0, sampleValue / Math.max(1.0, maxMetricValue)))
          anchors.bottom: parent.bottom

          color: isSelected
            ? Color.accent
            : (index === timelineRepeater.count - 1
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
              historyPage.scrubIndex = index
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

  // Step scrub controls and timing details
  Row {
    width: historyPage.width
    height: Style.space(22)
    spacing: Style.space(6)

    // Step -1s button
    Rectangle {
      width: Style.space(28)
      height: Style.space(20)
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
      width: Style.space(28)
      height: Style.space(20)
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

    // Timing summary text
    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: historyPage.timingLabel()
      color: historyPage.isLive ? historyPage.foreground : Color.accent
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.bodySmall
      font.bold: !historyPage.isLive
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
    // For IO, calculate maximum throughput observed in the current buffer
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
    var prefix = isLive ? "LIVE (NOW): " : ("-" + offset + "s (" + selectedSample.timestamp + "): ")
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
}
