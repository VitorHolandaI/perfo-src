import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui

Column {
  id: historyPage

  property var history: []
  property var liveSample: null
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

  readonly property bool isLive: scrubIndex < 0 || (history.length > 0 && effectiveIndex >= history.length - 1)

  readonly property var selectedSample: {
    if (isLive && liveSample) return liveSample
    if (history.length === 0 || effectiveIndex < 0) return liveSample
    return history[effectiveIndex]
  }

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
            color: historyPage.metric === modelData ? "#000000" : historyPage.foreground
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
            color: historyPage.zoomLabel === modelData ? "#000000" : historyPage.foreground
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
        color: historyPage.isCustomZoom ? "#000000" : historyPage.foreground
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

  }

  // Row 2: Playback & Action Controls (REC, PLAY, LIVE, jump & step buttons)
  Row {
    width: historyPage.width
    height: Style.space(20)
    spacing: Style.space(4)

    // REC / FREEZE button
    Rectangle {
      width: Style.space(50)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: "transparent"
      border.color: historyPage.isRecording ? Color.urgent : historyPage.foreground
      border.width: 1

      Row {
        anchors.centerIn: parent
        spacing: 4
        Rectangle {
          width: 6
          height: 6
          radius: 3
          anchors.verticalCenter: parent.verticalCenter
          color: historyPage.isRecording ? Color.urgent : historyPage.foreground
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

    // PLAY REC / PAUSE button
    Rectangle {
      width: playButtonText.implicitWidth + Style.space(12)
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: historyPage.isPlaying ? Color.accent : "transparent"
      border.color: historyPage.foreground
      border.width: 1

      PlainText {
        id: playButtonText
        anchors.centerIn: parent
        text: historyPage.isPlaying ? "PAUSE" : "PLAY REC"
        color: historyPage.isPlaying ? "#000000" : historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: historyPage.isPlaying
      }

      MouseArea {
        anchors.fill: parent
        onClicked: historyPage.togglePlayback()
      }
    }

    // Jump back button (<<)
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
        text: "<<"
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: true
      }

      MouseArea {
        anchors.fill: parent
        onClicked: historyPage.jumpTimeline(-1)
      }
    }

    // Step -1s button (<)
    Rectangle {
      width: Style.space(18)
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
        onClicked: historyPage.stepTimeline(-1)
      }
    }

    // Step +1s button (>)
    Rectangle {
      width: Style.space(18)
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
        onClicked: historyPage.stepTimeline(1)
      }
    }

    // Jump forward button (>>)
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
        text: ">>"
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: true
      }

      MouseArea {
        anchors.fill: parent
        onClicked: historyPage.jumpTimeline(1)
      }
    }

    // Jump to LIVE button
    Rectangle {
      width: Style.space(36)
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
        color: historyPage.isLive ? "#000000" : historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: historyPage.isLive
      }

      MouseArea {
        anchors.fill: parent
        onClicked: historyPage.jumpToLive()
      }
    }

    // Export to TXT + JSON button
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

    // Prominent Timer badge
    Rectangle {
      height: Style.space(18)
      width: timerBadgeText.implicitWidth + Style.space(12)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: historyPage.isLive ? "transparent" : Color.accent
      border.color: Color.accent
      border.width: 1

      PlainText {
        id: timerBadgeText
        anchors.centerIn: parent
        text: historyPage.isPlaying
          ? ("▶ REPLAY " + historyPage.timerClockString())
          : ("⏱ " + historyPage.timerClockString())
        color: historyPage.isLive ? Color.accent : "#000000"
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: true
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
      width: historyPage.width - Style.space(350)
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
        }
      }
    }

    // Vertical needle cursor line running through the bars
    Rectangle {
      id: needleCursor
      visible: historyPage.history.length > 0
      width: 2
      height: parent.height - 4
      anchors.verticalCenter: parent.verticalCenter
      color: Color.accent
      z: 5
      x: {
        var w = parent.width - 6
        var r = historyPage.rulerCursorRatio()
        return 3 + Math.round(r * (w - 2))
      }
    }

    // Interactive drag and click scrubbing on the entire bars area
    MouseArea {
      anchors.fill: parent
      z: 10
      hoverEnabled: true
      preventStealing: true
      onClicked: function(mouse) {
        historyPage.scrubToX(mouse.x - 3, width - 6)
      }
      onPositionChanged: function(mouse) {
        if (pressed) {
          historyPage.scrubToX(mouse.x - 3, width - 6)
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

  // Timeline Ruler and Time Axis: |------|------|------|< with timestamps below
  Item {
    id: timelineRuler
    width: historyPage.width
    height: Style.space(34)

    // Ruler track with horizontal line and tick marks
    Item {
      id: rulerTrack
      width: parent.width
      height: Style.space(14)
      anchors.top: parent.top

      // Horizontal baseline
      Rectangle {
        width: parent.width
        height: 1
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.35
      }

      // Major tick: Start (0%)
      Rectangle {
        width: 1
        height: Style.space(10)
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.75
      }

      // Minor tick: 12.5%
      Rectangle {
        width: 1
        height: Style.space(5)
        x: Math.round(parent.width * 0.125)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.3
      }

      // Major tick: 25%
      Rectangle {
        width: 1
        height: Style.space(8)
        x: Math.round(parent.width * 0.25)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.6
      }

      // Minor tick: 37.5%
      Rectangle {
        width: 1
        height: Style.space(5)
        x: Math.round(parent.width * 0.375)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.3
      }

      // Major tick: 50% (Center)
      Rectangle {
        width: 1
        height: Style.space(10)
        x: Math.round(parent.width * 0.5)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.75
      }

      // Minor tick: 62.5%
      Rectangle {
        width: 1
        height: Style.space(5)
        x: Math.round(parent.width * 0.625)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.3
      }

      // Major tick: 75%
      Rectangle {
        width: 1
        height: Style.space(8)
        x: Math.round(parent.width * 0.75)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.6
      }

      // Minor tick: 87.5%
      Rectangle {
        width: 1
        height: Style.space(5)
        x: Math.round(parent.width * 0.875)
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.3
      }

      // Major tick: End (100% / Live)
      Rectangle {
        width: 1
        height: Style.space(10)
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.foreground
        opacity: 0.75
      }

      // Dynamic cursor marker: ▲ pointing up to the tick track
      Item {
        id: rulerCursorMarker
        visible: historyPage.history.length > 0
        width: Style.space(14)
        height: parent.height
        x: Math.round(historyPage.rulerCursorRatio() * (parent.width - width))

        PlainText {
          anchors.centerIn: parent
          text: "▲"
          color: Color.accent
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }
      }

      // Drag and click mouse area on ruler
      MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        preventStealing: true
        onClicked: function(mouse) {
          historyPage.scrubToX(mouse.x, width)
        }
        onPositionChanged: function(mouse) {
          if (pressed) {
            historyPage.scrubToX(mouse.x, width)
          }
        }
      }
    }

    // Timestamps row below the ruler track
    Item {
      width: parent.width
      height: Style.space(16)
      anchors.top: rulerTrack.bottom

      // Start time (left)
      PlainText {
        anchors.left: parent.left
        anchors.verticalCenter: parent.verticalCenter
        text: historyPage.rulerStartTime()
        color: historyPage.foreground
        opacity: 0.65
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
      }

      // Cursor position time badge (center / floating)
      Rectangle {
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        height: Style.space(16)
        width: cursorTimeBadgeText.implicitWidth + Style.space(10)
        radius: Style.cornerRadius
        color: historyPage.isLive ? "transparent" : Color.accent
        border.color: Color.accent
        border.width: 1
        visible: historyPage.history.length > 0

        PlainText {
          id: cursorTimeBadgeText
          anchors.centerIn: parent
          text: historyPage.selectedSample
            ? ((historyPage.isLive ? "LIVE " : "") + historyPage.selectedSample.timestamp + " (" + historyPage.timerOffsetLabel() + ")")
            : "--"
          color: historyPage.isLive ? Color.accent : "#000000"
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }
      }

      // End time (right / Live)
      PlainText {
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        text: historyPage.rulerEndTime()
        color: historyPage.foreground
        opacity: 0.65
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        horizontalAlignment: Text.AlignRight
      }
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
    PlainText { width: Style.space(60); text: historyPage.metricHeader(); color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption; horizontalAlignment: Text.AlignRight }
    PlainText { width: Style.space(48); text: historyPage.secondaryHeader(); color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption; horizontalAlignment: Text.AlignRight }
    PlainText { width: parent.width - Style.space(300); text: "COMMAND"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
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
        width: Style.space(60)
        text: historyPage.metricCellText(modelData)
        color: Color.accent
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }

      PlainText {
        width: Style.space(48)
        text: historyPage.secondaryCellText(modelData)
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }

      PlainText {
        width: parent.width - Style.space(300)
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
    text: historyPage.metric === "GPU" ? "no active GPU processes for this sample" : "no process activity recorded for this sample"
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
    if (zoomLabel === "ALL") return Math.max(1, history.length)
    return customSpanSeconds
  }

  function rulerCursorRatio() {
    if (!history || history.length === 0) return 1.0
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var sliceCount = history.length - startIdx
    if (sliceCount <= 1) return 1.0
    var eff = effectiveIndex
    if (eff < startIdx) return 0.0
    return Math.max(0.0, Math.min(1.0, (eff - startIdx) / (sliceCount - 1)))
  }

  function rulerStartTime() {
    if (!history || history.length === 0) {
      if (liveSample && liveSample.timestamp) return liveSample.timestamp + " (+0s)"
      return "--:--:--"
    }
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var sample = history[startIdx]
    var t = (sample && sample.timestamp) ? sample.timestamp : "--:--:--"
    return t + " (+0s)"
  }

  function rulerEndTime() {
    var sample = (isLive && liveSample) ? liveSample : (history && history.length > 0 ? history[history.length - 1] : null)
    if (!sample) return "--:--:--"
    var span = currentZoomSeconds()
    var totalSpan = Math.min(span, Math.max(1, history.length))
    var t = sample.timestamp ? sample.timestamp : "--:--:--"
    return t + " (+" + formatDuration(totalSpan) + " LIVE)"
  }

  function timerClockString() {
    if (!history || history.length === 0) return "00:00 / 00:00"
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var totalSpan = Math.min(span, Math.max(1, history.length))
    var eff = effectiveIndex
    var elapsed = isLive
      ? totalSpan
      : Math.min(totalSpan, Math.max(0, Math.round(((eff - startIdx) / Math.max(1, history.length - 1 - startIdx)) * totalSpan)))
    return formatTimerClock(elapsed) + " / " + formatTimerClock(totalSpan)
  }

  function timerOffsetLabel() {
    if (!history || history.length === 0 || !selectedSample) return "+0s"
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var totalSpan = Math.min(span, Math.max(1, history.length))
    var eff = effectiveIndex
    var elapsed = isLive
      ? totalSpan
      : Math.min(totalSpan, Math.max(0, Math.round(((eff - startIdx) / Math.max(1, history.length - 1 - startIdx)) * totalSpan)))
    return "+" + formatDuration(elapsed)
  }

  function formatTimerClock(seconds) {
    var s = Math.max(0, Math.floor(Number(seconds) || 0))
    var m = Math.floor(s / 60)
    var remS = s % 60
    if (m >= 60) {
      var h = Math.floor(m / 60)
      var remM = m % 60
      return ("0" + h).slice(-2) + ":" + ("0" + remM).slice(-2) + ":" + ("0" + remS).slice(-2)
    }
    return ("0" + m).slice(-2) + ":" + ("0" + remS).slice(-2)
  }

  function scrubToX(mouseX, totalWidth) {
    if (!history || history.length === 0) return
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var sliceCount = history.length - startIdx
    if (sliceCount <= 0) return
    var ratio = Math.max(0.0, Math.min(1.0, mouseX / Math.max(1, totalWidth)))
    var target = Math.round(startIdx + ratio * (sliceCount - 1))
    if (target >= history.length - 1) {
      scrubIndex = -1
    } else {
      scrubIndex = Math.max(0, target)
    }
    isPlaying = false
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

  function stepTimeline(delta) {
    if (!history || history.length === 0) return
    var target = effectiveIndex + delta
    if (target >= 0 && target < history.length) {
      scrubIndex = target
      isPlaying = false
    }
  }

  function jumpStepSeconds() {
    var span = currentZoomSeconds()
    if (span <= 120) return 10
    if (span <= 900) return 30
    if (span <= 3600) return 60
    return 300
  }

  function jumpTimeline(direction) {
    if (!history || history.length === 0) return
    var step = jumpStepSeconds() * (direction < 0 ? -1 : 1)
    var target = effectiveIndex + step
    if (target < 0) target = 0
    if (target >= history.length - 1) {
      scrubIndex = -1
    } else {
      scrubIndex = target
    }
    isPlaying = false
  }

  function togglePlayback() {
    if (isPlaying) {
      isPlaying = false
    } else {
      if (effectiveIndex >= history.length - 1) {
        scrubIndex = Math.max(0, history.length - currentZoomSeconds())
      }
      isPlaying = true
    }
  }

  function jumpToLive() {
    scrubIndex = -1
    isPlaying = false
  }

  function metricHeader() {
    if (metric === "CPU") return "CPU%"
    if (metric === "MEM") return "MEM%"
    if (metric === "GPU") return "GPU%"
    if (metric === "IO") return "IO RATE"
    return metric
  }

  function secondaryHeader() {
    if (metric === "GPU") return "VRAM"
    return "RAM"
  }

  function timingLabel() {
    if (!selectedSample) return "No history recorded yet"
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, history.length - span)
    var totalSpan = Math.min(span, Math.max(1, history.length))
    var eff = effectiveIndex
    var elapsed = isLive
      ? totalSpan
      : Math.min(totalSpan, Math.max(0, Math.round(((eff - startIdx) / Math.max(1, history.length - 1 - startIdx)) * totalSpan)))
    var durStr = "+" + formatDuration(elapsed)
    var prefix = ""
    if (isPlaying) {
      prefix = "REPLAY [" + durStr + "] (" + selectedSample.timestamp + "): "
    } else if (isLive) {
      prefix = isRecording ? ("LIVE [" + durStr + "]: ") : ("LIVE (PAUSED REC) [" + selectedSample.timestamp + "]: ")
    } else {
      prefix = durStr + " (" + selectedSample.timestamp + "): "
    }
    return prefix + "CPU " + selectedSample.cpu + "% | MEM " + selectedSample.mem + "% | IO " + formatRate(selectedSample.read_bps + selectedSample.write_bps) + " | GPU " + selectedSample.gpu + "%"
  }

  function sortedProcesses() {
    if (!selectedSample || !selectedSample.processes) return []
    var list = selectedSample.processes.slice()
    if (metric === "GPU") {
      var gpuList = list.filter(function(p) {
        return (Number(p.gpu_percent) || 0) > 0 || (Number(p.vram_bytes) || 0) > 0
      })
      gpuList.sort(function(a, b) {
        var diff = (Number(b.gpu_percent) || 0) - (Number(a.gpu_percent) || 0)
        if (diff !== 0) return diff
        return (Number(b.vram_bytes) || 0) - (Number(a.vram_bytes) || 0)
      })
      return gpuList.slice(0, 5)
    }
    if (metric === "IO") {
      list.sort(function(a, b) {
        var aIo = (Number(a.read_bps) || 0) + (Number(a.write_bps) || 0)
        var bIo = (Number(b.read_bps) || 0) + (Number(b.write_bps) || 0)
        return bIo - aIo
      })
      return list.slice(0, 5)
    }
    if (metric === "MEM") {
      list.sort(function(a, b) { return (Number(b.mem_bytes) || 0) - (Number(a.mem_bytes) || 0) })
      return list.slice(0, 5)
    }
    list.sort(function(a, b) { return (Number(b.cpu_percent) || 0) - (Number(a.cpu_percent) || 0) })
    return list.slice(0, 5)
  }

  function metricCellText(proc) {
    if (metric === "MEM") {
      return formatBytes(proc.mem_bytes)
    }
    if (metric === "GPU") {
      return (Number(proc.gpu_percent) || 0) > 0 ? (Math.round(proc.gpu_percent) + "%") : "--"
    }
    if (metric === "IO") {
      var totalIo = (Number(proc.read_bps) || 0) + (Number(proc.write_bps) || 0)
      return totalIo > 0 ? formatRate(totalIo) : "--"
    }
    return Math.round(Number(proc.cpu_percent) || 0) + "%"
  }

  function secondaryCellText(proc) {
    if (metric === "GPU") {
      return (Number(proc.vram_bytes) || 0) > 0 ? formatBytes(proc.vram_bytes) : "--"
    }
    return formatBytes(proc.mem_bytes)
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

  function computeSummaryStats() {
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

    return {
      peakCpu: peakCpu,
      peakCpuTime: peakCpuTime,
      peakCpuProc: peakCpuProc,
      peakMem: peakMem,
      peakMemTime: peakMemTime,
      peakIo: peakIo,
      peakIoTime: peakIoTime,
      peakGpu: peakGpu,
      peakGpuTime: peakGpuTime,
      avgCpu: avgCpu,
      avgMem: avgMem
    }
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
    var baseName = "perfo-history-" + datePart + "-" + timePart
    var txtFullPath = "~/" + baseName + ".txt"
    var jsonFullPath = "~/" + baseName + ".json"
    var report = generateExportText(now)
    var reportJson = generateExportJson(now)

    exportProc.targetFilename = baseName + ".{txt,json}"
    exportProc.command = [
      "python3",
      "-c",
      "import sys, pathlib; pathlib.Path(sys.argv[1]).expanduser().write_text(sys.argv[3], encoding='utf-8'); pathlib.Path(sys.argv[2]).expanduser().write_text(sys.argv[4], encoding='utf-8')",
      txtFullPath,
      jsonFullPath,
      report,
      reportJson
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

  function generateExportJson(dateObj) {
    var sample = historyPage.selectedSample
    var out = {
      version: "1.0",
      generator: "perfo",
      generated_at: dateObj.toISOString(),
      metric_focus: historyPage.metric,
      zoom_span: historyPage.zoomLabel,
      zoom_seconds: historyPage.currentZoomSeconds(),
      total_recorded_samples: historyPage.history.length,
      current_scrub_index: historyPage.effectiveIndex,
      is_live: historyPage.isLive,
      selected_sample: sample ? {
        timestamp: sample.timestamp,
        cpu_percent: sample.cpu,
        mem_percent: sample.mem,
        read_bps: sample.read_bps,
        write_bps: sample.write_bps,
        io_mb: sample.io_mb,
        gpu_percent: sample.gpu,
        processes: sample.processes || []
      } : null,
      summary: computeSummaryStats(),
      timeline: []
    }

    var step = Math.max(1, Math.floor(historyPage.history.length / 1000))
    for (var i = 0; i < historyPage.history.length; i += step) {
      var s = historyPage.history[i]
      out.timeline.push({
        index: i,
        timestamp: s.timestamp,
        cpu_percent: s.cpu,
        mem_percent: s.mem,
        io_mb: s.io_mb,
        read_bps: s.read_bps,
        write_bps: s.write_bps,
        gpu_percent: s.gpu,
        top_process: (s.processes && s.processes.length > 0) ? (s.processes[0].name || s.processes[0].cmd || "") : ""
      })
    }
    return JSON.stringify(out, null, 2)
  }

  function generateExportText(dateObj) {
    var lines = []
    var border = "================================================================================"
    var subBorder = "--------------------------------------------------------------------------------"
    var stats = computeSummaryStats()

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
      lines.push(padRight("PID", 8) + " | " + padLeft(historyPage.metricHeader(), 8) + " | " + padLeft(historyPage.secondaryHeader(), 10) + " | " + padRight("PROCESS", 18) + " | COMMAND")
      lines.push(subBorder)
      var procs = historyPage.sortedProcesses()
      for (var i = 0; i < procs.length; i++) {
        var p = procs[i]
        var pName = historyPage.cleanName(p.name || p.cmd, p.pid)
        lines.push(
          padRight(p.pid, 8) + " | " +
          padLeft(historyPage.metricCellText(p), 8) + " | " +
          padLeft(historyPage.secondaryCellText(p), 10) + " | " +
          padRight(pName, 18) + " | " +
          (p.cmd || pName)
        )
      }
      if (procs.length === 0) {
        lines.push("No active " + historyPage.metric + " processes recorded for this sample.")
      }
    } else {
      lines.push("No sample data available.")
    }
    lines.push("")

    lines.push(border)
    lines.push("                   2. TIMELINE METRICS & PEAKS SUMMARY")
    lines.push(border)
    lines.push("Peak CPU Usage       : " + stats.peakCpu + "% at " + stats.peakCpuTime + " (Top: " + stats.peakCpuProc + ")")
    lines.push("Peak Memory Usage    : " + stats.peakMem + "% at " + stats.peakMemTime)
    lines.push("Peak Disk I/O Rate   : " + stats.peakIo.toFixed(1) + " MB/s at " + stats.peakIoTime)
    lines.push("Peak GPU Usage       : " + stats.peakGpu + "% at " + stats.peakGpuTime)
    lines.push("Average CPU Usage    : " + stats.avgCpu + "%")
    lines.push("Average Memory Usage : " + stats.avgMem + "%")
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
