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
  property int customMinutes: 5
  property int customSpanSeconds: 300
  property bool customInputOpen: false
  property string exportStatus: ""
  property var loadedHistory: []
  property string loadedSessionId: ""
  property string loadedSessionPath: ""
  property string loadedSessionTitle: ""
  property string loadedSessionDuration: ""
  property var windowProcessCache: ({})
  property int cachedWindowStart: -1
  property int cachedWindowEnd: -1
  property bool isFetchingWindow: false
  property int pendingFetchIndex: -1
  property var savedRecordings: []
  property bool showSessionsMenu: false
  property string sessionNotification: ""
  property real currentMaxMetric: 100.0
  property int playbackSpeed: 1
  property var cachedBars: []
  property var currentTopProcs: []
  property real lastTopProcsUpdateTime: 0

  property bool isSessionRecording: false
  property var sessionRecordBuffer: []
  property int targetRecordSeconds: 120

  signal toggleRecordingRequested()
  signal requestCapacity(int samples)

  readonly property bool inputActiveFocus: customInputOpen
  readonly property bool isCustomZoom: zoomLabel === "CUSTOM"
  readonly property var activeHistory: isSessionRecording ? sessionRecordBuffer : (loadedSessionId.length > 0 ? loadedHistory : history)
  readonly property string perfoBinPath: {
    var p = String(Qt.resolvedUrl("bin/perfo"))
    if (p.indexOf("file://") === 0) p = p.substring(7)
    return p
  }

  readonly property int effectiveIndex: {
    if (activeHistory.length === 0) return -1
    if (scrubIndex < 0 || scrubIndex >= activeHistory.length) return activeHistory.length - 1
    return scrubIndex
  }

  readonly property int selectedBarIndex: {
    if (!activeHistory || activeHistory.length === 0 || effectiveIndex < 0) return -1
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var sliceCount = activeHistory.length - startIdx
    if (sliceCount <= 0) return -1
    var eff = effectiveIndex
    if (eff < startIdx || eff >= activeHistory.length) return -1
    var barCount = cachedBars.length > 0 ? cachedBars.length : 100
    if (sliceCount <= barCount) return eff - startIdx
    return Math.min(barCount - 1, Math.floor(((eff - startIdx) / sliceCount) * barCount))
  }

  readonly property bool isLive: !isSessionRecording && loadedSessionId.length === 0 && (scrubIndex < 0 || (activeHistory.length > 0 && effectiveIndex >= activeHistory.length - 1))

  readonly property var selectedSample: {
    if (isLive && liveSample) return liveSample
    if (activeHistory.length === 0 || effectiveIndex < 0) return liveSample
    return activeHistory[effectiveIndex]
  }

  onLiveSampleChanged: {
    if (isSessionRecording && liveSample) {
      var nextBuf = sessionRecordBuffer.slice()
      nextBuf.push(liveSample)
      sessionRecordBuffer = nextBuf
      if (sessionRecordBuffer.length >= targetRecordSeconds) {
        historyPage.stopAndSaveSession()
      }
    }
  }

  onMetricChanged: {
    rebuildVisibleBars()
    updateTopProcesses(true)
  }
  onSelectedSampleChanged: updateTopProcesses(false)
  onZoomLabelChanged: rebuildVisibleBars()
  onCustomSpanSecondsChanged: rebuildVisibleBars()
  onLoadedSessionIdChanged: rebuildVisibleBars()
  onActiveHistoryChanged: {
    if (loadedSessionId.length === 0) {
      rebuildVisibleBars()
    }
  }

  spacing: Style.space(5)

  Timer {
    id: playbackTimer
    interval: 1000
    repeat: true
    running: historyPage.isPlaying && historyPage.activeHistory.length > 0
    onTriggered: {
      var step = historyPage.playbackSpeed || 1
      var next = historyPage.effectiveIndex + step
      if (next >= historyPage.activeHistory.length) {
        if (historyPage.loadedSessionId.length > 0) {
          historyPage.scrubIndex = historyPage.activeHistory.length - 1
        } else {
          historyPage.scrubIndex = -1
        }
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

  Timer {
    id: sessionNotificationTimer
    interval: 3500
    repeat: false
    onTriggered: historyPage.sessionNotification = ""
  }

  Timer {
    id: topProcsThrottleTimer
    interval: 60
    repeat: false
    onTriggered: {
      historyPage.lastTopProcsUpdateTime = Date.now()
      historyPage.currentTopProcs = historyPage.sortedProcesses()
    }
  }

  FileView {
    id: recordingFileReader
    printErrors: false
    blockLoading: true
  }

  Timer {
    id: inspectDebounceTimer
    interval: 80
    repeat: false
    onTriggered: {
      if (historyPage.loadedSessionId.length > 0 && historyPage.loadedSessionPath.length > 0) {
        historyPage.fetchProcessWindow(historyPage.effectiveIndex)
      }
    }
  }

  Process {
    id: loadTimelineProc
    property string pendingRecPath: ""
    property string pendingRecId: ""
    property bool pendingAutoPlay: false
    property bool pendingScrubToEnd: false
    property string outputBuffer: ""
    stdout: SplitParser {
      onRead: function(line) {
        loadTimelineProc.outputBuffer += line
      }
    }
    onExited: function(exitCode, exitStatus) {
      if (exitCode === 0 && outputBuffer.length > 0) {
        try {
          var data = JSON.parse(outputBuffer)
          historyPage.applyLoadedTimeline(data, pendingRecPath, pendingRecId, pendingAutoPlay, pendingScrubToEnd)
        } catch(e) {
          console.warn("Failed to parse timeline output:", e)
          historyPage.sessionNotification = "Failed to parse timeline"
          sessionNotificationTimer.restart()
        }
      } else {
        historyPage.sessionNotification = "Failed to load timeline"
        sessionNotificationTimer.restart()
      }
    }
  }

  Process {
    id: inspectProc
    property int requestedIndex: -1
    property string outputBuffer: ""
    stdout: SplitParser {
      onRead: function(line) {
        inspectProc.outputBuffer += line
      }
    }
    onExited: function(exitCode, exitStatus) {
      historyPage.isFetchingWindow = false
      if (exitCode === 0 && outputBuffer.length > 0) {
        try {
          var data = JSON.parse(outputBuffer)
          if (data && data.slices) {
            var cache = Object.assign({}, historyPage.windowProcessCache)
            for (var i = 0; i < data.slices.length; i++) {
              var sl = data.slices[i]
              cache[sl.index] = sl.processes || []
            }
            var keys = Object.keys(cache)
            if (keys.length > 250) {
              var cur = historyPage.effectiveIndex
              for (var k = 0; k < keys.length; k++) {
                var idx = Number(keys[k])
                if (Math.abs(idx - cur) > 100) {
                  delete cache[keys[k]]
                }
              }
            }
            historyPage.windowProcessCache = cache
            historyPage.cachedWindowStart = data.start_index
            historyPage.cachedWindowEnd = data.end_index
            historyPage.currentTopProcs = historyPage.sortedProcesses()
          }
        } catch(e) {
          console.warn("Failed to parse inspect slices:", e)
        }
      }
      if (historyPage.pendingFetchIndex >= 0 && historyPage.pendingFetchIndex !== requestedIndex) {
        var nextIdx = historyPage.pendingFetchIndex
        historyPage.pendingFetchIndex = -1
        historyPage.fetchProcessWindow(nextIdx)
      }
    }
  }

  Process {
    id: listRecordingsProc
    command: [historyPage.perfoBinPath, "record", "list"]
    stdout: SplitParser {
      onRead: function(line) {
        try {
          historyPage.savedRecordings = JSON.parse(line)
        } catch(e) {
          console.warn("Failed to parse recordings list:", e)
        }
      }
    }
  }

  Process {
    id: saveRecordingProc
    property string lastSavedPath: ""
    property string lastSavedId: ""
    property string lastSavedDuration: ""
    property string payloadToSend: ""
    stdinEnabled: true
    onStarted: {
      if (payloadToSend.length > 0) {
        saveRecordingProc.write(payloadToSend + "\n")
        payloadToSend = ""
      }
    }
    stdout: SplitParser {
      onRead: function(line) {
        try {
          var res = JSON.parse(line)
          if (res && res.path) {
            saveRecordingProc.lastSavedPath = res.path
            saveRecordingProc.lastSavedId = res.id || ""
            saveRecordingProc.lastSavedDuration = res.duration || ""
          }
        } catch(e) {}
      }
    }
    onExited: function(exitCode, exitStatus) {
      if (exitCode === 0) {
        var dur = saveRecordingProc.lastSavedDuration.length > 0 ? saveRecordingProc.lastSavedDuration : "session"
        historyPage.sessionNotification = "Saved " + dur + " to disk!"
        historyPage.refreshRecordings()
        if (saveRecordingProc.lastSavedPath.length > 0) {
          historyPage.loadSession(saveRecordingProc.lastSavedPath, saveRecordingProc.lastSavedId, false, true)
        }
      } else {
        historyPage.sessionNotification = "Save failed (code " + exitCode + ")"
      }
      sessionNotificationTimer.restart()
    }
  }

  Process {
    id: deleteRecordingProc
    onExited: function(exitCode, exitStatus) {
      if (exitCode === 0) {
        historyPage.sessionNotification = "Deleted recording"
        historyPage.refreshRecordings()
      }
      sessionNotificationTimer.restart()
    }
  }

  function refreshRecordings() {
    listRecordingsProc.running = false
    listRecordingsProc.command = [historyPage.perfoBinPath, "record", "list"]
    listRecordingsProc.running = true
  }

  Component.onCompleted: {
    rebuildVisibleBars()
    updateTopProcesses(true)
    historyPage.refreshRecordings()
  }

  onVisibleChanged: {
    if (visible) {
      historyPage.refreshRecordings()
    }
  }

  onShowSessionsMenuChanged: {
    if (showSessionsMenu) {
      historyPage.refreshRecordings()
    }
  }

  Process {
    id: exportProc
    property string targetFilename: ""
    property string payloadToSend: ""
    stdinEnabled: true
    onStarted: {
      if (payloadToSend.length > 0) {
        exportProc.write(payloadToSend + "\n")
        payloadToSend = ""
      }
    }
    onExited: function(exitCode, exitStatus) {
      if (exitCode === 0) {
        historyPage.exportStatus = "Saved: ~/" + targetFilename
      } else {
        historyPage.exportStatus = "Export failed"
      }
      exportStatusTimer.restart()
    }
  }

  // Row 1: Title, metric selector, zoom presets, and custom span input
  Row {
    width: historyPage.width
    height: Style.space(22)
    spacing: Style.space(4)

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: "TIMELINE"
      color: historyPage.foreground
      opacity: 0.65
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: true
    }

    // Metric selector pills: CPU, MEM, IO, NET, GPU
    Row {
      spacing: Style.space(3)
      anchors.verticalCenter: parent.verticalCenter

      Repeater {
        model: ["CPU", "MEM", "IO", "NET", "GPU"]
        delegate: Rectangle {
          width: Style.space(30)
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

    Item { width: Style.space(2); height: 1 }

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: historyPage.isSessionRecording ? "REC TIME" : "SPAN"
      color: historyPage.isSessionRecording ? Color.urgent : historyPage.foreground
      opacity: historyPage.isSessionRecording ? 1.0 : 0.55
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: historyPage.isSessionRecording
    }

    // Duration presets: 2m, 5m, 10m, 15m (Locked during recording!)
    Row {
      spacing: Style.space(3)
      anchors.verticalCenter: parent.verticalCenter
      opacity: historyPage.isSessionRecording ? 0.35 : 1.0

      Repeater {
        model: ["2m", "5m", "10m", "15m"]
        delegate: Rectangle {
          width: Style.space(26)
          height: Style.space(18)
          radius: Style.cornerRadius
          color: (!historyPage.isCustomZoom && historyPage.zoomLabel === modelData) ? Color.accent : "transparent"
          border.color: historyPage.foreground
          border.width: 1
          opacity: (!historyPage.isCustomZoom && historyPage.zoomLabel === modelData) ? 1.0 : 0.55

          PlainText {
            anchors.centerIn: parent
            text: modelData
            color: (!historyPage.isCustomZoom && historyPage.zoomLabel === modelData) ? "#000000" : historyPage.foreground
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: (!historyPage.isCustomZoom && historyPage.zoomLabel === modelData)
          }

          MouseArea {
            anchors.fill: parent
            enabled: !historyPage.isSessionRecording
            onClicked: {
              historyPage.zoomLabel = modelData
              historyPage.customInputOpen = false
            }
          }
        }
      }
    }

    // Custom duration button & input
    Rectangle {
      id: customSpanBox
      width: historyPage.customInputOpen ? Style.space(92) : (historyPage.isCustomZoom ? Style.space(72) : Style.space(52))
      height: Style.space(18)
      radius: Style.cornerRadius
      anchors.verticalCenter: parent.verticalCenter
      color: historyPage.isCustomZoom ? Color.accent : "transparent"
      border.color: historyPage.customInputOpen ? Color.accent : historyPage.foreground
      border.width: 1
      opacity: historyPage.isSessionRecording ? 0.35 : 1.0

      // Editing mode: text input + 'm' + OK + Cancel
      Row {
        visible: historyPage.customInputOpen
        anchors.fill: parent
        anchors.margins: 1
        spacing: 2

        TextInput {
          id: customMinutesInput
          width: Style.space(30)
          height: parent.height
          color: historyPage.isCustomZoom ? "#000000" : historyPage.foreground
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          horizontalAlignment: TextInput.AlignHCenter
          verticalAlignment: TextInput.AlignVCenter
          selectByMouse: true
          maximumLength: 4
          validator: IntValidator { bottom: 1; top: 1440 }

          Keys.onEscapePressed: function(event) {
            historyPage.customInputOpen = false
            event.accepted = true
          }
          Keys.onReturnPressed: function(event) {
            historyPage.applyCustomMinutes(customMinutesInput.text)
            event.accepted = true
          }
        }

        PlainText {
          anchors.verticalCenter: parent.verticalCenter
          text: "m"
          color: historyPage.isCustomZoom ? "#000000" : historyPage.foreground
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
        }

        Rectangle {
          width: Style.space(16)
          height: parent.height
          radius: 2
          color: Color.accent
          PlainText {
            anchors.centerIn: parent
            text: "✓"
            color: "#000000"
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: true
          }
          MouseArea {
            anchors.fill: parent
            onClicked: historyPage.applyCustomMinutes(customMinutesInput.text)
          }
        }

        Rectangle {
          width: Style.space(16)
          height: parent.height
          radius: 2
          color: "transparent"
          border.color: historyPage.foreground
          border.width: 1
          PlainText {
            anchors.centerIn: parent
            text: "✕"
            color: historyPage.foreground
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
          }
          MouseArea {
            anchors.fill: parent
            onClicked: historyPage.customInputOpen = false
          }
        }
      }

      // Display mode: "CUSTOM" or "CUSTOM: 5m"
      PlainText {
        visible: !historyPage.customInputOpen
        anchors.centerIn: parent
        text: historyPage.isCustomZoom ? ("CUSTOM: " + historyPage.customMinutes + "m") : "CUSTOM"
        color: historyPage.isCustomZoom ? "#000000" : historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        font.bold: historyPage.isCustomZoom
      }

      MouseArea {
        anchors.fill: parent
        visible: !historyPage.customInputOpen
        enabled: !historyPage.isSessionRecording
        onClicked: {
          customMinutesInput.text = String(historyPage.customMinutes)
          historyPage.customInputOpen = true
          customMinutesInput.forceActiveFocus()
          customMinutesInput.selectAll()
        }
      }
    }
  }

  // Row 2: Playback & Action Controls (left) and Timer Badge (right)
  Item {
    width: historyPage.width
    height: Style.space(20)

    Row {
      anchors.left: parent.left
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(3)

      // REC / STOP button
      Rectangle {
        width: historyPage.isSessionRecording ? Style.space(52) : Style.space(46)
        height: Style.space(18)
        radius: Style.cornerRadius
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.isSessionRecording ? Color.urgent : "transparent"
        border.color: Color.urgent
        border.width: 1

        Row {
          anchors.centerIn: parent
          spacing: 3
          Rectangle {
            width: 6
            height: 6
            radius: 3
            anchors.verticalCenter: parent.verticalCenter
            color: historyPage.isSessionRecording ? "#ffffff" : Color.urgent
          }
          PlainText {
            anchors.verticalCenter: parent.verticalCenter
            text: historyPage.isSessionRecording ? "STOP" : "REC"
            color: historyPage.isSessionRecording ? "#ffffff" : Color.urgent
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: true
          }
        }

        MouseArea {
          anchors.fill: parent
          onClicked: historyPage.toggleSessionRecording()
        }
      }

      // PLAY REC / PAUSE button
      Rectangle {
        width: playButtonText.implicitWidth + Style.space(10)
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
          onClicked: {
            if (historyPage.isPlaying) {
              historyPage.togglePlayback()
            } else if (historyPage.loadedSessionId.length > 0) {
              historyPage.togglePlayback()
            } else if (!historyPage.showSessionsMenu) {
              historyPage.showSessionsMenu = true
            } else {
              historyPage.togglePlayback()
            }
          }
        }
      }

      // Playback speed selector button
      Rectangle {
        width: speedButtonText.implicitWidth + Style.space(8)
        height: Style.space(18)
        radius: Style.cornerRadius
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.playbackSpeed > 1 ? Color.accent : "transparent"
        border.color: historyPage.playbackSpeed > 1 ? Color.accent : historyPage.foreground
        border.width: 1

        PlainText {
          id: speedButtonText
          anchors.centerIn: parent
          text: historyPage.playbackSpeed + "x"
          color: historyPage.playbackSpeed > 1 ? "#000000" : historyPage.foreground
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: historyPage.playbackSpeed > 1
        }

        MouseArea {
          anchors.fill: parent
          onClicked: historyPage.cyclePlaybackSpeed()
        }
      }

      // SESSIONS selector toggle button
      Rectangle {
        width: sessionsButtonText.implicitWidth + Style.space(10)
        height: Style.space(18)
        radius: Style.cornerRadius
        anchors.verticalCenter: parent.verticalCenter
        color: (historyPage.showSessionsMenu || historyPage.loadedSessionId.length > 0) ? Color.accent : "transparent"
        border.color: (historyPage.showSessionsMenu || historyPage.loadedSessionId.length > 0) ? Color.accent : historyPage.foreground
        border.width: 1

        PlainText {
          id: sessionsButtonText
          anchors.centerIn: parent
          text: {
            var arrow = historyPage.showSessionsMenu ? " ▲" : " ▾"
            if (historyPage.loadedSessionId.length > 0) {
              var dur = historyPage.loadedSessionDuration ? historyPage.loadedSessionDuration : "REC"
              return "📁 " + dur + arrow
            }
            var count = (historyPage.savedRecordings && historyPage.savedRecordings.length > 0) ? " (" + historyPage.savedRecordings.length + ")" : ""
            return "📁 SESSIONS" + count + arrow
          }
          color: (historyPage.showSessionsMenu || historyPage.loadedSessionId.length > 0) ? "#000000" : historyPage.foreground
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: (historyPage.showSessionsMenu || historyPage.loadedSessionId.length > 0)
        }

        MouseArea {
          anchors.fill: parent
          onClicked: historyPage.showSessionsMenu = !historyPage.showSessionsMenu
        }
      }

      // Jump back button (<<)
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
        width: Style.space(16)
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
        width: Style.space(16)
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
        width: Style.space(18)
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
        width: Style.space(32)
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
        width: Style.space(42)
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
          cursorShape: Qt.PointingHandCursor
          enabled: !exportProc.running
          onClicked: historyPage.exportReport()
        }
      }
    }

    // Right side: Status notification and Prominent Timer badge anchored to right
    Row {
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(4)

      // Status notification badge (session notification or export status)
      Rectangle {
        visible: historyPage.sessionNotification.length > 0 || historyPage.exportStatus.length > 0
        height: Style.space(18)
        width: notifStatusText.implicitWidth + Style.space(10)
        radius: Style.cornerRadius
        anchors.verticalCenter: parent.verticalCenter
        color: Color.accent

        PlainText {
          id: notifStatusText
          anchors.centerIn: parent
          text: historyPage.sessionNotification.length > 0 ? historyPage.sessionNotification : historyPage.exportStatus
          color: "#000000"
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }
      }

      // Prominent Timer badge
      Rectangle {
        height: Style.space(18)
        width: timerBadgeText.implicitWidth + Style.space(10)
        radius: Style.cornerRadius
        anchors.verticalCenter: parent.verticalCenter
        color: historyPage.isSessionRecording ? Color.urgent : (historyPage.isLive ? "transparent" : Color.accent)
        border.color: historyPage.isSessionRecording ? Color.urgent : Color.accent
        border.width: 1

        PlainText {
          id: timerBadgeText
          anchors.centerIn: parent
          text: {
            if (historyPage.isSessionRecording) {
              return "● REC " + historyPage.timerClockString()
            }
            if (historyPage.loadedSessionId.length > 0) {
              return (historyPage.isPlaying ? "▶ REC " : "📁 REC ") + historyPage.timerClockString()
            }
            return historyPage.isPlaying
              ? ("▶ REPLAY " + historyPage.timerClockString())
              : ("⏱ " + historyPage.timerClockString())
          }
          color: (historyPage.isSessionRecording || !historyPage.isLive) ? "#000000" : Color.accent
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }
      }
    }
  }

  // Submenu panel for selecting which session to play, inspect, or manage
  Rectangle {
    id: sessionsPanel
    visible: historyPage.showSessionsMenu
    width: historyPage.width
    height: sessionsColumn.implicitHeight + Style.space(14)
    color: "transparent"
    border.color: Color.accent
    border.width: 1
    radius: Style.cornerRadius

    Column {
      id: sessionsColumn
      anchors.fill: parent
      anchors.margins: Style.space(6)
      spacing: Style.space(5)

      // Submenu Header
      Item {
        width: parent.width
        height: Style.space(20)

        PlainText {
          anchors.left: parent.left
          anchors.verticalCenter: parent.verticalCenter
          text: "SELECT RECORDING TO PLAY (MAX 5)"
          color: Color.accent
          font.family: historyPage.fontFamily
          font.pixelSize: Style.font.caption
          font.bold: true
        }

        // "+ SAVE CURRENT" button
        Rectangle {
          anchors.right: parent.right
          anchors.verticalCenter: parent.verticalCenter
          width: Style.space(100)
          height: Style.space(18)
          radius: Style.cornerRadius
          color: "transparent"
          border.color: Color.accent
          border.width: 1

          PlainText {
            anchors.centerIn: parent
            text: "+ SAVE CURRENT"
            color: Color.accent
            font.family: historyPage.fontFamily
            font.pixelSize: Style.font.caption
            font.bold: true
          }

          MouseArea {
            anchors.fill: parent
            onClicked: historyPage.saveCurrentSession()
          }
        }
      }

      // Divider line
      Rectangle {
        width: parent.width
        height: 1
        color: historyPage.foreground
        opacity: 0.2
      }

      // Item 0: Live Buffer (Current Active Session)
      Rectangle {
        width: parent.width
        height: Style.space(24)
        radius: Style.cornerRadius
        color: historyPage.loadedSessionId.length === 0 ? Qt.rgba(Color.accent.r, Color.accent.g, Color.accent.b, 0.15) : "transparent"
        border.color: historyPage.loadedSessionId.length === 0 ? Color.accent : Qt.rgba(historyPage.foreground.r, historyPage.foreground.g, historyPage.foreground.b, 0.2)
        border.width: 1

        Item {
          anchors.fill: parent
          anchors.leftMargin: Style.space(8)
          anchors.rightMargin: Style.space(6)

          Row {
            anchors.left: parent.left
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(8)

            Rectangle {
              width: 6
              height: 6
              radius: 3
              anchors.verticalCenter: parent.verticalCenter
              color: historyPage.loadedSessionId.length === 0 ? Color.accent : historyPage.foreground
              opacity: historyPage.loadedSessionId.length === 0 ? 1.0 : 0.5
            }

            PlainText {
              anchors.verticalCenter: parent.verticalCenter
              text: "LIVE BUFFER (Current Session)"
              color: historyPage.loadedSessionId.length === 0 ? Color.accent : historyPage.foreground
              font.family: historyPage.fontFamily
              font.pixelSize: Style.font.caption
              font.bold: historyPage.loadedSessionId.length === 0
            }

            PlainText {
              anchors.verticalCenter: parent.verticalCenter
              text: historyPage.formatDuration(historyPage.history.length) + " (" + historyPage.history.length + "s)"
              color: historyPage.foreground
              opacity: 0.6
              font.family: historyPage.fontFamily
              font.pixelSize: Style.font.caption
            }
          }

          Row {
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: Style.space(4)

            // PLAY button for live buffer
            Rectangle {
              width: Style.space(50)
              height: Style.space(16)
              radius: Style.cornerRadius
              color: Color.accent
              border.color: Color.accent
              border.width: 1

              PlainText {
                anchors.centerIn: parent
                text: "▶ PLAY"
                color: "#000000"
                font.family: historyPage.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: true
              }

              MouseArea {
                anchors.fill: parent
                onClicked: {
                  historyPage.jumpToLive()
                  historyPage.togglePlayback()
                  historyPage.showSessionsMenu = false
                }
              }
            }

            // SWITCH TO LIVE button
            Rectangle {
              width: Style.space(46)
              height: Style.space(16)
              radius: Style.cornerRadius
              color: historyPage.loadedSessionId.length === 0 ? Color.accent : "transparent"
              border.color: historyPage.loadedSessionId.length === 0 ? Color.accent : historyPage.foreground
              border.width: 1
              opacity: historyPage.loadedSessionId.length === 0 ? 1.0 : 0.75

              PlainText {
                anchors.centerIn: parent
                text: historyPage.loadedSessionId.length === 0 ? "ACTIVE" : "LIVE"
                color: historyPage.loadedSessionId.length === 0 ? "#000000" : historyPage.foreground
                font.family: historyPage.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: true
              }

              MouseArea {
                anchors.fill: parent
                onClicked: {
                  historyPage.jumpToLive()
                  historyPage.showSessionsMenu = false
                }
              }
            }
          }
        }
      }

      // Items 1..5: Saved recordings
      Repeater {
        model: historyPage.savedRecordings
        delegate: Rectangle {
          id: recItemBox
          width: sessionsColumn.width
          height: Style.space(24)
          radius: Style.cornerRadius
          readonly property bool isCurrent: historyPage.loadedSessionId === modelData.id
          color: isCurrent ? Qt.rgba(Color.accent.r, Color.accent.g, Color.accent.b, 0.15) : "transparent"
          border.color: isCurrent ? Color.accent : Qt.rgba(historyPage.foreground.r, historyPage.foreground.g, historyPage.foreground.b, 0.2)
          border.width: 1

          Item {
            anchors.fill: parent
            anchors.leftMargin: Style.space(8)
            anchors.rightMargin: Style.space(6)

            Row {
              id: recInfoRow
              anchors.left: parent.left
              anchors.right: recButtonsRow.left
              anchors.rightMargin: Style.space(6)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(6)

              PlainText {
                anchors.verticalCenter: parent.verticalCenter
                text: "📁 " + modelData.date + " " + modelData.time
                color: recItemBox.isCurrent ? Color.accent : historyPage.foreground
                font.family: historyPage.fontFamily
                font.pixelSize: Style.font.caption
                font.bold: recItemBox.isCurrent
              }

              PlainText {
                anchors.verticalCenter: parent.verticalCenter
                text: modelData.duration + " (" + modelData.sample_count + "s)"
                color: historyPage.foreground
                opacity: 0.6
                font.family: historyPage.fontFamily
                font.pixelSize: Style.font.caption
                elide: Text.ElideRight
              }
            }

            // Right side buttons: [▶ PLAY] [LOAD] [✕]
            Row {
              id: recButtonsRow
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(4)

              // LOAD button
              Rectangle {
                width: Style.space(48)
                height: Style.space(16)
                radius: Style.cornerRadius
                color: Color.accent
                border.color: Color.accent
                border.width: 1

                PlainText {
                  anchors.centerIn: parent
                  text: "LOAD"
                  color: "#000000"
                  font.family: historyPage.fontFamily
                  font.pixelSize: Style.font.caption
                  font.bold: true
                }

                MouseArea {
                  anchors.fill: parent
                  onClicked: {
                    historyPage.loadSession(modelData.path, modelData.id, false)
                    historyPage.showSessionsMenu = false
                  }
                }
              }

              // DELETE [✕] button
              Rectangle {
                width: Style.space(18)
                height: Style.space(16)
                radius: Style.cornerRadius
                color: "transparent"
                border.color: Color.urgent
                border.width: 1

                PlainText {
                  anchors.centerIn: parent
                  text: "✕"
                  color: Color.urgent
                  font.family: historyPage.fontFamily
                  font.pixelSize: Style.font.caption
                  font.bold: true
                }

                MouseArea {
                  anchors.fill: parent
                  onClicked: historyPage.deleteSession(modelData.id)
                }
              }
            }
          }
        }
      }

      // Empty placeholder when no saved files exist yet
      PlainText {
        visible: (!historyPage.savedRecordings || historyPage.savedRecordings.length === 0)
        width: parent.width - Style.space(16)
        text: "No saved recordings yet. Click '+ SAVE CURRENT' to save."
        color: historyPage.foreground
        opacity: 0.5
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.caption
        horizontalAlignment: Text.AlignHCenter
        anchors.horizontalCenter: parent.horizontalCenter
        wrapMode: Text.WordWrap
      }
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
        model: historyPage.cachedBars

        delegate: Rectangle {
          id: barDelegate
          readonly property bool isSelected: index === historyPage.selectedBarIndex
          readonly property real sampleValue: Number(modelData.value) || 0

          width: Math.max(1, (barsRow.width / Math.max(1, timelineRepeater.count)) - 1)
          height: Math.max(2, barsRow.height * Math.min(1.0, sampleValue / Math.max(1.0, historyPage.currentMaxMetric)))
          anchors.bottom: parent.bottom

          color: isSelected
            ? Color.accent
            : (modelData.rawIndex === historyPage.activeHistory.length - 1
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
      visible: historyPage.activeHistory.length > 0
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
      visible: historyPage.activeHistory.length === 0
      text: historyPage.loadedSessionId.length > 0 ? "empty recording" : "collecting history snapshots..."
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
        visible: historyPage.activeHistory.length > 0
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
        visible: historyPage.activeHistory.length > 0

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

  // Section title and metrics summary for process inspector
  Row {
    width: historyPage.width
    height: Style.space(16)
    spacing: Style.space(6)

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: {
        if (historyPage.loadedSessionId.length > 0) {
          var dur = historyPage.loadedSessionDuration ? historyPage.loadedSessionDuration : "REC"
          return historyPage.isPlaying ? ("REPLAY (" + dur + ")") : ("RECORDING (" + dur + ")")
        }
        return historyPage.isPlaying
          ? "REPLAY PROCESSES"
          : (historyPage.isLive ? (historyPage.isRecording ? "LIVE PROCESSES" : "LIVE (PAUSED REC)") : "HISTORICAL PROCESSES")
      }
      color: historyPage.foreground
      opacity: 0.7
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: true
    }

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: "│"
      color: historyPage.foreground
      opacity: 0.35
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
    }

    PlainText {
      anchors.verticalCenter: parent.verticalCenter
      text: historyPage.sampleMetricsSummary()
      color: Color.accent
      font.family: historyPage.fontFamily
      font.pixelSize: Style.font.caption
      font.bold: true
      elide: Text.ElideRight
      width: historyPage.width - Style.space(180)
    }
  }

  // Process table headers
  Row {
    width: historyPage.width
    spacing: Style.space(8)

    PlainText { width: Style.space(48); text: "PID"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
    PlainText { width: Style.space(110); text: "PROCESS"; color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
    PlainText { width: Style.space(68); text: historyPage.metricHeader(); color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption; horizontalAlignment: Text.AlignRight }
    PlainText { width: Style.space(68); text: historyPage.secondaryHeader(); color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption; horizontalAlignment: Text.AlignRight }
    PlainText { width: parent.width - Style.space(330); text: historyPage.detailHeader(); color: historyPage.foreground; opacity: 0.55; font.family: historyPage.fontFamily; font.pixelSize: Style.font.caption }
  }

  // Process rows at selected sample
  Repeater {
    model: historyPage.currentTopProcs

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
        width: Style.space(110)
        text: historyPage.cleanName(modelData.name || modelData.cmd, modelData.pid)
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        font.bold: true
        elide: Text.ElideRight
      }

      PlainText {
        width: Style.space(68)
        text: historyPage.metricCellText(modelData)
        color: Color.accent
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }

      PlainText {
        width: Style.space(68)
        text: historyPage.secondaryCellText(modelData)
        color: historyPage.foreground
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        horizontalAlignment: Text.AlignRight
      }

      PlainText {
        width: parent.width - Style.space(330)
        text: historyPage.detailCellText(modelData)
        color: historyPage.foreground
        opacity: historyPage.metric === "NET" ? 0.95 : 0.7
        font.family: historyPage.fontFamily
        font.pixelSize: Style.font.bodySmall
        elide: Text.ElideRight
      }
    }
  }

  PlainText {
    visible: historyPage.currentTopProcs.length === 0
    text: {
      if (historyPage.metric === "GPU") return "no active GPU processes for this sample"
      if (historyPage.metric === "NET") return "no active network socket processes for this sample"
      return "no process activity recorded for this sample"
    }
    color: historyPage.foreground
    opacity: 0.55
    font.family: historyPage.fontFamily
    font.pixelSize: Style.font.bodySmall
  }

  // Helper functions
  function currentZoomSeconds() {
    if (zoomLabel === "2m") return 120
    if (zoomLabel === "5m") return 300
    if (zoomLabel === "10m") return 600
    if (zoomLabel === "15m") return 900
    if (zoomLabel === "CUSTOM") return customSpanSeconds
    return 120
  }

  function rulerCursorRatio() {
    if (!activeHistory || activeHistory.length === 0) return 1.0
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var sliceCount = activeHistory.length - startIdx
    if (sliceCount <= 1) return 1.0
    var eff = effectiveIndex
    if (eff < startIdx) return 0.0
    return Math.max(0.0, Math.min(1.0, (eff - startIdx) / (sliceCount - 1)))
  }

  function rulerStartTime() {
    if (!activeHistory || activeHistory.length === 0) {
      if (liveSample && liveSample.timestamp) return liveSample.timestamp + " (+0s)"
      return "--:--:--"
    }
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var sample = activeHistory[startIdx]
    var t = (sample && sample.timestamp) ? sample.timestamp : "--:--:--"
    return t + " (+0s)"
  }

  function rulerEndTime() {
    var sample = (isLive && liveSample) ? liveSample : (activeHistory && activeHistory.length > 0 ? activeHistory[activeHistory.length - 1] : null)
    if (!sample) return "--:--:--"
    var span = currentZoomSeconds()
    var totalSpan = (loadedSessionId.length > 0) ? activeHistory.length : Math.min(span, Math.max(1, activeHistory.length))
    var t = sample.timestamp ? sample.timestamp : "--:--:--"
    var suffix = isLive ? " (+" + formatDuration(totalSpan) + " LIVE)" : " (+" + formatDuration(totalSpan) + ")"
    return t + suffix
  }

  function timerClockString() {
    if (isSessionRecording) {
      return formatTimerClock(sessionRecordBuffer.length) + " / " + formatTimerClock(targetRecordSeconds)
    }
    if (!activeHistory || activeHistory.length === 0) return "00:00 / 00:00"
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var totalSpan = (loadedSessionId.length > 0) ? activeHistory.length : Math.min(span, Math.max(1, activeHistory.length))
    var eff = effectiveIndex
    var elapsed = isLive
      ? totalSpan
      : Math.min(totalSpan, Math.max(0, Math.round(((eff - startIdx) / Math.max(1, activeHistory.length - 1 - startIdx)) * totalSpan)))
    return formatTimerClock(elapsed) + " / " + formatTimerClock(totalSpan)
  }

  function timerOffsetLabel() {
    if (!activeHistory || activeHistory.length === 0 || !selectedSample) return "+0s"
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var totalSpan = (loadedSessionId.length > 0) ? activeHistory.length : Math.min(span, Math.max(1, activeHistory.length))
    var eff = effectiveIndex
    var elapsed = isLive
      ? totalSpan
      : Math.min(totalSpan, Math.max(0, Math.round(((eff - startIdx) / Math.max(1, activeHistory.length - 1 - startIdx)) * totalSpan)))
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
    if (!activeHistory || activeHistory.length === 0) return
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var sliceCount = activeHistory.length - startIdx
    if (sliceCount <= 0) return
    var ratio = Math.max(0.0, Math.min(1.0, mouseX / Math.max(1, totalWidth)))
    var target = Math.round(startIdx + ratio * (sliceCount - 1))
    if (target >= activeHistory.length - 1 && loadedSessionId.length === 0) {
      scrubIndex = -1
    } else {
      scrubIndex = Math.max(0, Math.min(activeHistory.length - 1, target))
    }
    isPlaying = false
  }

  function applyCustomMinutes(rawText) {
    var str = String(rawText || "").trim()
    var num = parseInt(str, 10)
    if (!isNaN(num) && num >= 1 && num <= 1440 && /^\d+$/.test(str)) {
      customMinutes = num
      customSpanSeconds = num * 60
      zoomLabel = "CUSTOM"
      customInputOpen = false
      historyPage.requestCapacity(customSpanSeconds)
      rebuildVisibleBars()
    } else {
      sessionNotification = "Enter valid integer minutes"
      sessionNotificationTimer.restart()
    }
  }

  function rebuildVisibleBars() {
    if (!activeHistory || activeHistory.length === 0) {
      currentMaxMetric = 100.0
      cachedBars = []
      return
    }
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var sliceCount = activeHistory.length - startIdx
    var maxBars = 100
    var maxPeak = 1.0

    if (sliceCount <= maxBars) {
      var bars = []
      for (var i = startIdx; i < activeHistory.length; i++) {
        var rawSample = activeHistory[i]
        var val = metricValue(rawSample)
        if (val > maxPeak) maxPeak = val
        bars.push({
          rawIndex: i,
          value: val
        })
      }
      updateMaxMetric(maxPeak)
      cachedBars = bars
      return
    }

    // Downsample into buckets for long time spans (such as 40m, 1h, 2h, 40h)
    var bucketSize = sliceCount / maxBars
    var downsampled = []
    for (var b = 0; b < maxBars; b++) {
      var bStart = Math.floor(startIdx + b * bucketSize)
      var bEnd = Math.min(activeHistory.length, Math.floor(startIdx + (b + 1) * bucketSize))
      if (bStart >= bEnd) continue

      var peakVal = 0
      var peakIdx = bStart
      for (var k = bStart; k < bEnd; k++) {
        var v = metricValue(activeHistory[k])
        if (v >= peakVal) {
          peakVal = v
          peakIdx = k
        }
      }
      if (peakVal > maxPeak) maxPeak = peakVal
      downsampled.push({
        rawIndex: peakIdx,
        startIndex: bStart,
        endIndex: bEnd,
        value: peakVal
      })
    }
    updateMaxMetric(maxPeak)
    cachedBars = downsampled
  }

  function updateMaxMetric(peak) {
    if (metric === "CPU" || metric === "MEM" || metric === "GPU") {
      currentMaxMetric = 100.0
    } else if (metric === "NET") {
      currentMaxMetric = Math.max(1048576.0, peak)
    } else if (metric === "IO") {
      currentMaxMetric = Math.max(10.0, peak)
    } else {
      currentMaxMetric = Math.max(1.0, peak)
    }
  }

  function visibleBars() {
    return cachedBars
  }

  function metricValue(sample) {
    if (!sample) return 0
    if (metric === "CPU") return Number(sample.cpu) || 0
    if (metric === "MEM") return Number(sample.mem) || 0
    if (metric === "IO") return Number(sample.io_mb) || 0
    if (metric === "NET") return (Number(sample.net_rx_bps) || 0) + (Number(sample.net_tx_bps) || 0)
    if (metric === "GPU") return Number(sample.gpu) || 0
    return 0
  }

  function maxMetric(type) {
    return currentMaxMetric
  }

  function stepTimeline(delta) {
    if (!activeHistory || activeHistory.length === 0) return
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var current = effectiveIndex
    var step = (Math.abs(delta) === 1 && playbackSpeed > 1) ? (delta * playbackSpeed) : delta
    var target = current + step
    target = Math.max(startIdx, Math.min(activeHistory.length - 1, target))
    if (target >= activeHistory.length - 1 && loadedSessionId.length === 0) {
      scrubIndex = -1
    } else {
      scrubIndex = target
    }
    isPlaying = false
  }

  function jumpStepSeconds() {
    var span = currentZoomSeconds()
    if (span <= 120) return 10
    if (span <= 900) return 30
    if (span <= 3600) return 60
    if (span <= 7200) return 120
    return 300
  }

  function jumpTimeline(direction) {
    if (!activeHistory || activeHistory.length === 0) return
    var span = currentZoomSeconds()
    var startIdx = (loadedSessionId.length > 0) ? 0 : Math.max(0, activeHistory.length - span)
    var step = jumpStepSeconds() * (direction < 0 ? -1 : 1)
    var current = effectiveIndex
    var target = current + step
    target = Math.max(startIdx, Math.min(activeHistory.length - 1, target))
    if (target >= activeHistory.length - 1 && loadedSessionId.length === 0) {
      scrubIndex = -1
    } else {
      scrubIndex = target
    }
    isPlaying = false
  }

  function cyclePlaybackSpeed() {
    var speeds = [1, 2, 5, 10, 30, 60, 120, 300]
    var idx = speeds.indexOf(playbackSpeed)
    if (idx < 0 || idx === speeds.length - 1) {
      playbackSpeed = speeds[0]
    } else {
      playbackSpeed = speeds[idx + 1]
    }
  }

  function togglePlayback() {
    if (isPlaying) {
      isPlaying = false
    } else {
      if (activeHistory.length === 0) return
      if (scrubIndex < 0 || scrubIndex >= activeHistory.length - 1) {
        scrubIndex = 0
      }
      isPlaying = true
    }
  }

  function jumpToLive() {
    isSessionRecording = false
    sessionRecordBuffer = []
    loadedSessionId = ""
    loadedSessionPath = ""
    loadedSessionTitle = ""
    loadedSessionDuration = ""
    loadedHistory = []
    windowProcessCache = ({})
    cachedWindowStart = -1
    cachedWindowEnd = -1
    if (inspectProc.running) inspectProc.running = false
    if (loadTimelineProc.running) loadTimelineProc.running = false
    scrubIndex = -1
    isPlaying = false
    zoomLabel = "2m"
    rebuildVisibleBars()
    updateTopProcesses(true)
  }

  function toggleSessionRecording() {
    if (isSessionRecording) {
      stopAndSaveSession()
    } else {
      startSessionRecording()
    }
  }

  function startSessionRecording() {
    sessionRecordBuffer = []
    targetRecordSeconds = currentZoomSeconds()
    isSessionRecording = true
    loadedSessionId = ""
    loadedSessionPath = ""
    loadedSessionTitle = ""
    loadedSessionDuration = ""
    loadedHistory = []
    windowProcessCache = ({})
    cachedWindowStart = -1
    cachedWindowEnd = -1
    if (inspectProc.running) inspectProc.running = false
    if (loadTimelineProc.running) loadTimelineProc.running = false
    scrubIndex = -1
    isPlaying = false
    sessionNotification = "Recording " + formatDuration(targetRecordSeconds) + "..."
    sessionNotificationTimer.restart()
  }

  function stopAndSaveSession() {
    if (!isSessionRecording) return
    isSessionRecording = false
    if (sessionRecordBuffer.length >= 1) {
      saveSessionData(sessionRecordBuffer, sessionRecordBuffer.length)
    } else {
      sessionRecordBuffer = []
      sessionNotification = "Recording cancelled"
      sessionNotificationTimer.restart()
    }
  }

  function stopSessionRecording() {
    stopAndSaveSession()
  }

  function saveSessionData(buf, targetSecs) {
    if (!buf || buf.length === 0) return
    var actualDur = buf.length
    var payload = {
      samples: buf,
      duration_seconds: actualDur,
      metric_focus: "ALL"
    }
    var jsonStr = JSON.stringify(payload)
    if (saveRecordingProc.running) {
      saveRecordingProc.running = false
    }
    saveRecordingProc.payloadToSend = jsonStr
    saveRecordingProc.command = [
      historyPage.perfoBinPath,
      "record",
      "save"
    ]
    saveRecordingProc.running = true
  }

  function saveCurrentSession() {
    var buf = (loadedSessionId.length > 0 ? loadedHistory : history)
    if (!buf || buf.length === 0) {
      sessionNotification = "Buffer is empty"
      sessionNotificationTimer.restart()
      return
    }
    saveSessionData(buf, buf.length)
  }

  function fetchProcessWindow(idx) {
    if (!loadedSessionPath || loadedSessionPath.length === 0) return
    if (isFetchingWindow) {
      pendingFetchIndex = idx
      return
    }
    isFetchingWindow = true
    inspectProc.outputBuffer = ""
    inspectProc.requestedIndex = idx
    inspectProc.command = [
      historyPage.perfoBinPath,
      "record",
      "inspect",
      historyPage.loadedSessionPath,
      String(idx),
      "25"
    ]
    inspectProc.running = true
  }

  function applyLoadedTimeline(data, recPath, recId, autoPlay, scrubToEnd) {
    if (!data || !data.samples || data.samples.length === 0) {
      sessionNotification = "Recording has no samples"
      sessionNotificationTimer.restart()
      return false
    }
    isSessionRecording = false
    sessionRecordBuffer = []
    loadedHistory = data.samples
    loadedSessionId = data.id || recId
    loadedSessionPath = recPath
    loadedSessionTitle = (data.date || "") + " " + (data.time || "")
    loadedSessionDuration = data.duration_label || ""
    windowProcessCache = ({})
    cachedWindowStart = -1
    cachedWindowEnd = -1
    customSpanSeconds = data.samples.length
    customMinutes = Math.max(1, Math.ceil(data.samples.length / 60))
    zoomLabel = "CUSTOM"
    scrubIndex = scrubToEnd ? (data.samples.length - 1) : 0
    isPlaying = autoPlay
    if (scrubToEnd) {
      sessionNotification = "Saved " + (data.duration_label || "") + " session to disk!"
    } else {
      sessionNotification = "Loaded " + (data.duration_label || "") + " session"
    }
    sessionNotificationTimer.restart()
    rebuildVisibleBars()
    updateTopProcesses(true)
    fetchProcessWindow(scrubIndex >= 0 ? scrubIndex : (data.samples.length - 1))
    return true
  }

  function loadSession(recPath, recId, autoPlay, scrubToEnd) {
    var timelinePath = ""
    if (recPath.endsWith(".json")) {
      timelinePath = recPath.substring(0, recPath.lastIndexOf(".")) + ".timeline.json"
    } else {
      timelinePath = recPath + ".timeline.json"
    }

    recordingFileReader.path = ""
    recordingFileReader.path = timelinePath
    var str = recordingFileReader.text()
    if (str && str.length > 0) {
      try {
        var data = JSON.parse(str)
        if (data && data.samples && data.samples.length > 0) {
          return applyLoadedTimeline(data, recPath, recId, autoPlay, scrubToEnd)
        }
      } catch(e) {
        console.warn("Timeline cache parse failed, generating:", e)
      }
    }

    loadTimelineProc.pendingRecPath = recPath
    loadTimelineProc.pendingRecId = recId
    loadTimelineProc.pendingAutoPlay = autoPlay || false
    loadTimelineProc.pendingScrubToEnd = scrubToEnd || false
    loadTimelineProc.outputBuffer = ""
    loadTimelineProc.command = [
      historyPage.perfoBinPath,
      "record",
      "timeline",
      recPath
    ]
    loadTimelineProc.running = true
    return true
  }

  function deleteSession(recId) {
    if (loadedSessionId === recId) {
      jumpToLive()
    }
    deleteRecordingProc.command = [historyPage.perfoBinPath, "record", "delete", recId]
    deleteRecordingProc.running = true
  }

  function metricHeader() {
    if (metric === "CPU") return "CPU%"
    if (metric === "MEM") return "MEM%"
    if (metric === "GPU") return "GPU%"
    if (metric === "IO") return "IO READ"
    if (metric === "NET") return "IN (RX)"
    return metric
  }

  function secondaryHeader() {
    if (metric === "GPU") return "VRAM"
    if (metric === "NET") return "OUT (TX)"
    if (metric === "IO") return "IO WRITE"
    return "RAM"
  }

  function detailHeader() {
    if (metric === "NET") return "TOTAL EXCHANGED / CONNS"
    if (metric === "IO") return "RAM / COMMAND"
    return "COMMAND"
  }

  function timingLabel() {
    if (!selectedSample) return "No history recorded yet"
    var span = currentZoomSeconds()
    var startIdx = Math.max(0, activeHistory.length - span)
    var totalSpan = Math.min(span, Math.max(1, activeHistory.length))
    var eff = effectiveIndex
    var elapsed = isLive
      ? totalSpan
      : Math.min(totalSpan, Math.max(0, Math.round(((eff - startIdx) / Math.max(1, activeHistory.length - 1 - startIdx)) * totalSpan)))
    var durStr = "+" + formatDuration(elapsed)
    var prefix = ""
    if (isSessionRecording) {
      prefix = "REC [" + formatTimerClock(sessionRecordBuffer.length) + " / " + formatTimerClock(targetRecordSeconds) + "]: "
    } else if (isPlaying) {
      prefix = "REPLAY [" + durStr + "] (" + selectedSample.timestamp + "): "
    } else if (isLive) {
      prefix = isRecording ? ("LIVE [" + durStr + "]: ") : ("LIVE (PAUSED REC) [" + selectedSample.timestamp + "]: ")
    } else {
      prefix = durStr + " (" + selectedSample.timestamp + "): "
    }
    var netBps = (Number(selectedSample.net_rx_bps) || 0) + (Number(selectedSample.net_tx_bps) || 0)
    return prefix + "CPU " + selectedSample.cpu + "% │ MEM " + selectedSample.mem + "% │ IO " + formatRate(selectedSample.read_bps + selectedSample.write_bps) + " │ NET " + formatRate(netBps) + " │ GPU " + selectedSample.gpu + "%"
  }

  function sampleMetricsSummary() {
    if (!selectedSample) return "No data recorded"
    var cpuVal = Math.round(Number(selectedSample.cpu) || 0)
    var memVal = Math.round(Number(selectedSample.mem) || 0)
    var ioVal = (Number(selectedSample.read_bps) || 0) + (Number(selectedSample.write_bps) || 0)
    var netVal = (Number(selectedSample.net_rx_bps) || 0) + (Number(selectedSample.net_tx_bps) || 0)
    var gpuVal = Math.round(Number(selectedSample.gpu) || 0)
    return "CPU " + cpuVal + "% │ MEM " + memVal + "% │ IO " + formatRate(ioVal) + " │ NET " + formatRate(netVal) + " │ GPU " + gpuVal + "%"
  }

  function updateTopProcesses(immediate) {
    if (loadedSessionId.length > 0 && loadedSessionPath.length > 0) {
      var eff = effectiveIndex
      if (windowProcessCache && windowProcessCache[eff]) {
        currentTopProcs = sortedProcesses()
        if (isPlaying && (eff > cachedWindowEnd - 5 || eff < cachedWindowStart + 5)) {
          var targetPrefetch = isPlaying ? (eff + 20) : eff
          fetchProcessWindow(targetPrefetch)
        }
        return
      }
      currentTopProcs = sortedProcesses()
      inspectDebounceTimer.restart()
      return
    }

    var now = Date.now()
    if (immediate || (now - lastTopProcsUpdateTime > 100)) {
      lastTopProcsUpdateTime = now
      topProcsThrottleTimer.stop()
      currentTopProcs = sortedProcesses()
    } else {
      topProcsThrottleTimer.restart()
    }
  }

  function sortedProcesses() {
    if (!selectedSample) return []
    var list = null
    if (selectedSample.processes && selectedSample.processes.length > 0) {
      list = selectedSample.processes.slice()
    } else if (loadedSessionId.length > 0 && windowProcessCache && windowProcessCache[effectiveIndex]) {
      list = windowProcessCache[effectiveIndex].slice()
    }

    if (!list || list.length === 0) {
      if (selectedSample.top_process && selectedSample.top_process.length > 0) {
        return [{
          pid: "--",
          name: selectedSample.top_process,
          cmd: selectedSample.top_process,
          cpu_percent: selectedSample.cpu || 0,
          mem_bytes: 0,
          read_bps: selectedSample.read_bps || 0,
          write_bps: selectedSample.write_bps || 0,
          net_rx_bps: selectedSample.net_rx_bps || 0,
          net_tx_bps: selectedSample.net_tx_bps || 0,
          gpu_percent: selectedSample.gpu || 0
        }]
      }
      return []
    }
    if (metric === "NET") {
      var netList = list.filter(function(p) {
        return (Number(p.net_rx_bps) || 0) > 0 || (Number(p.net_tx_bps) || 0) > 0 ||
               (Number(p.net_rx_bytes) || 0) > 0 || (Number(p.net_tx_bytes) || 0) > 0 ||
               (Number(p.total_sockets) || 0) > 0 || (Number(p.tcp_est) || 0) > 0 || (Number(p.udp) || 0) > 0
      })
      netList.sort(function(a, b) {
        var aRate = (Number(a.net_rx_bps) || 0) + (Number(a.net_tx_bps) || 0)
        var bRate = (Number(b.net_rx_bps) || 0) + (Number(b.net_tx_bps) || 0)
        if (bRate !== aRate) return bRate - aRate
        var aBytes = (Number(a.net_rx_bytes) || 0) + (Number(a.net_tx_bytes) || 0)
        var bBytes = (Number(b.net_rx_bytes) || 0) + (Number(b.net_tx_bytes) || 0)
        if (bBytes !== aBytes) return bBytes - aBytes
        var diff = (Number(b.tcp_est) || 0) - (Number(a.tcp_est) || 0)
        if (diff !== 0) return diff
        return (Number(b.total_sockets) || 0) - (Number(a.total_sockets) || 0)
      })
      return netList.slice(0, 5)
    }
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
      return (Number(proc.read_bps) || 0) > 0 ? formatRate(proc.read_bps) : "--"
    }
    if (metric === "NET") {
      var rxBps = Number(proc.net_rx_bps) || 0
      if (rxBps > 0) return formatRate(rxBps)
      var rxBytes = Number(proc.net_rx_bytes) || 0
      if (rxBytes > 0) return formatBytes(rxBytes)
      return "--"
    }
    return Math.round(Number(proc.cpu_percent) || 0) + "%"
  }

  function secondaryCellText(proc) {
    if (metric === "GPU") {
      return (Number(proc.vram_bytes) || 0) > 0 ? formatBytes(proc.vram_bytes) : "--"
    }
    if (metric === "IO") {
      return (Number(proc.write_bps) || 0) > 0 ? formatRate(proc.write_bps) : "--"
    }
    if (metric === "NET") {
      var txBps = Number(proc.net_tx_bps) || 0
      if (txBps > 0) return formatRate(txBps)
      var txBytes = Number(proc.net_tx_bytes) || 0
      if (txBytes > 0) return formatBytes(txBytes)
      return "--"
    }
    return formatBytes(proc.mem_bytes)
  }

  function detailCellText(proc) {
    if (metric === "NET") {
      var rxB = Number(proc.net_rx_bytes) || 0
      var txB = Number(proc.net_tx_bytes) || 0
      var totB = rxB + txB
      var parts = []
      if (totB > 0) {
        parts.push("Tot " + formatBytes(totB))
      }
      if ((Number(proc.tcp_est) || 0) > 0) {
        parts.push(proc.tcp_est + " est")
      } else if ((Number(proc.total_sockets) || 0) > 0) {
        parts.push(proc.total_sockets + " sock")
      }
      if (parts.length === 0 && proc.cmd) {
        return String(proc.cmd)
      }
      return parts.join(" │ ")
    }
    if (metric === "IO") {
      var ramStr = formatBytes(proc.mem_bytes)
      return ramStr + (proc.cmd ? (" │ " + String(proc.cmd)) : "")
    }
    return String(proc.cmd || "")
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
    var v = Number(bytes)
    if (!isFinite(v) || v <= 0) return "--"
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

    for (var k = 0; k < historyPage.activeHistory.length; k++) {
      var s = historyPage.activeHistory[k]
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

    var avgCpu = historyPage.activeHistory.length > 0 ? (totalCpu / historyPage.activeHistory.length).toFixed(1) : "0"
    var avgMem = historyPage.activeHistory.length > 0 ? (totalMem / historyPage.activeHistory.length).toFixed(1) : "0"

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
    if (!activeHistory || activeHistory.length === 0) {
      exportStatus = "No history to export"
      exportStatusTimer.restart()
      return
    }
    if (exportProc.running) {
      exportProc.running = false
    }
    var now = new Date()
    var datePart = now.getFullYear() +
      ("0" + (now.getMonth() + 1)).slice(-2) +
      ("0" + now.getDate()).slice(-2)
    var timePart = ("0" + now.getHours()).slice(-2) +
      ("0" + now.getMinutes()).slice(-2) +
      ("0" + now.getSeconds()).slice(-2)
    var baseName = "perfo-history-" + datePart + "-" + timePart

    var report = generateExportText(now)
    var reportJsonStr = generateExportJson(now)
    var reportJsonObj = null
    try {
      reportJsonObj = JSON.parse(reportJsonStr)
    } catch(e) {
      reportJsonObj = {}
    }

    var payload = JSON.stringify({
      text: report,
      json: reportJsonObj
    })

    exportProc.targetFilename = baseName + ".{txt,json}"
    exportProc.payloadToSend = payload
    exportProc.command = [
      historyPage.perfoBinPath,
      "export",
      baseName
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
      total_recorded_samples: historyPage.activeHistory.length,
      current_scrub_index: historyPage.effectiveIndex,
      is_live: historyPage.isLive,
      selected_sample: sample ? {
        timestamp: sample.timestamp,
        cpu_percent: sample.cpu,
        mem_percent: sample.mem,
        read_bps: sample.read_bps,
        write_bps: sample.write_bps,
        io_mb: sample.io_mb,
        net_rx_bps: sample.net_rx_bps,
        net_tx_bps: sample.net_tx_bps,
        net_rate: sample.net_rate,
        gpu_percent: sample.gpu,
        processes: sample.processes || []
      } : null,
      summary: computeSummaryStats(),
      timeline: []
    }

    var step = Math.max(1, Math.floor(historyPage.activeHistory.length / 1000))
    for (var i = 0; i < historyPage.activeHistory.length; i += step) {
      var s = historyPage.activeHistory[i]
      out.timeline.push({
        index: i,
        timestamp: s.timestamp,
        cpu_percent: s.cpu,
        mem_percent: s.mem,
        io_mb: s.io_mb,
        read_bps: s.read_bps,
        write_bps: s.write_bps,
        net_rx_bps: s.net_rx_bps,
        net_tx_bps: s.net_tx_bps,
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
    lines.push("Total Recorded Time  : " + historyPage.formatDuration(historyPage.activeHistory.length) + " (" + historyPage.activeHistory.length + " samples)")
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
      lines.push("Total Network Rate   : " + historyPage.formatRate((Number(sample.net_rx_bps) || 0) + (Number(sample.net_tx_bps) || 0)) + " (RX: " + historyPage.formatRate(sample.net_rx_bps) + ", TX: " + historyPage.formatRate(sample.net_tx_bps) + ")")
      lines.push("GPU Usage            : " + sample.gpu + "%")
      lines.push("")
      lines.push("Active Processes at this timing:")
      lines.push(padRight("PID", 8) + " | " + padLeft(historyPage.metricHeader(), 10) + " | " + padLeft(historyPage.secondaryHeader(), 10) + " | " + padRight("PROCESS", 18) + " | " + historyPage.detailHeader())
      lines.push(subBorder)
      var procs = historyPage.sortedProcesses()
      for (var i = 0; i < procs.length; i++) {
        var p = procs[i]
        var pName = historyPage.cleanName(p.name || p.cmd, p.pid)
        lines.push(
          padRight(p.pid, 8) + " | " +
          padLeft(historyPage.metricCellText(p), 10) + " | " +
          padLeft(historyPage.secondaryCellText(p), 10) + " | " +
          padRight(pName, 18) + " | " +
          historyPage.detailCellText(p)
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
    var step = Math.max(1, Math.floor(historyPage.activeHistory.length / 100))
    for (var j = 0; j < historyPage.activeHistory.length; j += step) {
      var sm = historyPage.activeHistory[j]
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
