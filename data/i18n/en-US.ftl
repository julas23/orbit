# Orbit — English (source and fallback for every other language).
#
# Translating: copy this file to <locale>.ftl (e.g. de-DE.ftl), translate the
# text after each "=", keep the ids and the { $placeholders }, and add the
# file to CATALOGS in crates/orbit-app/src/i18n.rs.
# Syntax: https://projectfluent.org/fluent/guide/

## Clock

clock-tooltip-year = week { $week } · day { $day } of the year

## Session

session-poweroff = power off
session-reboot = reboot
session-logout = log out
session-poweroff-question = Power off the computer?
session-reboot-question = Restart the computer?
session-logout-question = End the graphical session?
session-menu = session
session-yes = Yes
session-no = No
session-notification-title = Session
session-before-failed = Getting ready to leave did not finish.
    Leave anyway?
session-leave-anyway = Leave anyway
session-cancel = Cancel
session-choose = What do you want to do?

## Application menu

menu-label = Applications
menu-sublabel = Search and launch
menu-command = Run command
menu-search = Search applications…
menu-power = Power off, reboot or log out
menu-run = No application found. Enter runs “{ $command }”.
menu-category-all = All
menu-category-utilities = Accessories
menu-category-development = Development
menu-category-education = Education and science
menu-category-games = Games
menu-category-graphics = Graphics
menu-category-internet = Internet
menu-category-multimedia = Multimedia
menu-category-office = Office
menu-category-settings = Settings
menu-category-system = System
menu-category-other = Other

## Window list

windows-show = Show (was minimized)
windows-minimize = Minimize
windows-maximize = Maximize
windows-restore = Restore
windows-area = Area { $area }
windows-close = Close
windows-place-area = area { $area }
windows-place-minimized = minimized

## Workspaces

workspaces-tooltip = area { $area } — on every screen (Super+{ $area })

## Volume and brightness

volume-tooltip = volume: { $percent }%{ $muted ->
        [yes] {" "}(muted)
       *[no] {""}
    } — click the icon for the mixer, right click to mute
brightness-tooltip = brightness: { $percent }%

## Network

network-none = no network
network-none-tooltip = no active connection
network-no-address = no address
network-kind-wifi = Wi-Fi
network-kind-wired = wired
network-kind-vpn = VPN
network-menu-networking = Networking enabled
network-menu-wifi = Wi-Fi enabled
network-menu-edit = Edit connections…
network-quick-wifi = Wi-Fi networks
network-tooltip = Click: networks · Right click: connections

## Keyboard

keyboard-next = { $name }, of:
keyboard-tooltip = Click: next · Right click: choose · Wheel: walk through

## Battery

battery-none = no battery
battery-uptime = up for { $uptime }
battery-tooltip = battery { $percent }%, { $state }{ $remaining }
battery-remaining = {" "}— { $time } left
battery-charging = charging
battery-discharging = discharging
battery-full = charged
battery-pending = waiting to charge
battery-hint = Click: details and power profile
battery-profile = Power profile
battery-profile-saver = Power saver
battery-profile-balanced = Balanced
battery-profile-performance = Performance

## Weather

weather-tooltip = { $description } in { $place }
    low { $min }°C · high { $max }°C · sun { $sunrise }–{ $sunset }
    feels like { $feels }°C · humidity { $humidity }%
    wind { $wind } km/h { $direction } · pressure { $pressure } hPa
    clouds { $clouds }% · UV index { $uv }
    moon: { $moon } ({ $lit }% lit)
    { $season }
forecast-tooltip = { $weekday } { $date } — { $description }, low { $min }°C, high { $max }°C

# 16-point compass rose, from north clockwise, separated by spaces.
weather-compass = N NNE NE ENE E ESE SE SSE S SSW SW WSW W WNW NW NNW

weather-code-0 = Clear sky
weather-code-1 = Mainly sunny
weather-code-2 = Partly cloudy
weather-code-3 = Overcast
weather-code-45 = Fog
weather-code-48 = Freezing fog
weather-code-51 = Light drizzle
weather-code-53 = Drizzle
weather-code-55 = Heavy drizzle
weather-code-56 = Freezing drizzle
weather-code-61 = Light rain
weather-code-63 = Rain
weather-code-65 = Heavy rain
weather-code-66 = Freezing rain
weather-code-71 = Light snow
weather-code-73 = Snow
weather-code-75 = Heavy snow
weather-code-80 = Rain showers
weather-code-81 = Heavy showers
weather-code-82 = Violent showers
weather-code-85 = Snow showers
weather-code-95 = Thunderstorm
weather-code-96 = Thunderstorm with hail
weather-code-unknown = —

moon-new = new moon
moon-waxing-crescent = waxing crescent
moon-first-quarter = first quarter
moon-waxing-gibbous = waxing gibbous
moon-full = full moon
moon-waning-gibbous = waning gibbous
moon-last-quarter = last quarter
moon-waning-crescent = waning crescent

# The calendar's title: "Primavera 2026".
calendar-title = { $season } { $year }
season-winter = winter
season-spring = spring
season-summer = summer
season-autumn = autumn

## Dock

dock-open = Open
dock-recent = Recent
dock-edit = Edit dock
dock-edit-done = Done editing
dock-add-launcher = Add launcher…
dock-add-appimage = Add AppImage…
dock-add-separator = Add separator
dock-search = Search applications

## Calculator

calc-title = Calculator
calc-profile-standard = Standard
calc-profile-scientific = Scientific
calc-profile-financial = Financial
# The financial calculator's display when an operation fails (the HP 12c's error numbers).
calc-hp-error = Error { $code }
calc-angle = Unit of angles: degrees (DEG) or radians (RAD). Click to switch.
calc-profile-subnet = Subnet
# Number notation: the decimal separator and the digit-group separator
# (leave the group separator empty for no grouping).
calc-decimal-separator = .
calc-group-separator = ,
calc-error-syntax = Incomplete or invalid expression
calc-error-division-by-zero = Cannot divide by zero
calc-error-domain = Outside the function's domain
calc-error-overflow = Number too large
calc-subnet-prefix = Prefix length (mask bits)
calc-subnet-hint = Type an address, such as 10.0.0.199/24
calc-subnet-bad-address = Not an IPv4 address
calc-subnet-bad-mask = Invalid prefix or mask
calc-subnet-address = Address
calc-subnet-mask = Mask
calc-subnet-wildcard = Wildcard
calc-subnet-network = Network
calc-subnet-broadcast = Broadcast
calc-subnet-first = First host
calc-subnet-last = Last host
calc-subnet-hosts = Usable hosts
calc-subnet-size = Addresses
calc-subnet-class = Class
calc-subnet-kind = Kind
calc-subnet-binary-address = Address (binary)
calc-subnet-binary-mask = Mask (binary)
calc-subnet-hex-address = Address (hex)
calc-subnet-hex-mask = Mask (hex)
calc-subnet-none = none (/31 and /32)
calc-kind-private = private (RFC 1918)
calc-kind-shared = shared, CGNAT (RFC 6598)
calc-kind-loopback = loopback
calc-kind-link-local = link-local
calc-kind-multicast = multicast
calc-kind-reserved = reserved
calc-kind-broadcast = broadcast
calc-kind-public = public

## Clipboard

clip-title = Clipboard
clip-search = Search the history
clip-empty = Nothing copied yet
clip-no-match = Nothing found
clip-clear = Clear history
clip-count = { $count ->
        [one] 1 item
       *[other] { $count } items
    }
clip-image = image { $width } × { $height }
clip-image-unreadable = image (unreadable)
clip-ago-now = now
clip-ago-minutes = { $n } min ago
clip-ago-hours = { $n } h ago
clip-ago-days = { $n } d ago

## Screen capture

blink-title = Screen capture
blink-mode-screen = Screen
blink-mode-window = Window
blink-mode-area = Area
blink-delay = Delay (seconds)
blink-cursor = Include the pointer
blink-take = Capture
blink-saved = Screen captured
blink-saved-in = { $size } · on the clipboard
blink-copy = Copy
blink-copied = Copied
blink-save-as = Save as…
blink-failed = The capture failed
blink-no-window = No focused window to capture.
# Start of the saved file's name: screenshot-2026-10-19_22-50-12.png
blink-file-prefix = screenshot

## Image viewer

view-title = Orbit-View
view-empty = No image open.
    Drop an image here or choose one.
view-open = Open…
view-filter-images = Images
view-failed = Could not open { $name }.
    { $reason }
view-unknown-format = Unrecognised image format.
view-previous = Previous
view-next = Next
view-zoom-out = Zoom out
view-zoom-in = Zoom in
view-fit = Fit to window
view-actual-size = Actual size
view-turn-left = Turn left
view-turn-right = Turn right
view-mirror-horizontal = Mirror horizontally
view-mirror-vertical = Mirror vertically
view-save = Save the rotation into the file
view-save-unsupported = This format cannot be saved; only JPEG and PNG
view-saved = Saved
view-save-failed = Could not save: { $reason }
view-copy = Copy the image
view-copied = Image copied
view-info = Information
view-slideshow = Slideshow
view-fullscreen = Full screen
view-trash = Move to the trash
view-trashed = { $name } moved to the trash
view-trash-failed = Could not move to the trash: { $reason }
view-info-name = Name
view-info-folder = Folder
view-info-format = Format
view-info-dimensions = Dimensions
view-info-file-size = File size
view-info-modified = Modified
view-info-taken = Taken
view-info-camera = Camera
view-info-exposure = Exposure
view-info-software = Software
view-info-place = Place
view-info-coordinate = { $degrees }°{ $minutes }′{ $seconds }″ { $side }
view-info-north = N
view-info-south = S
view-info-east = E
view-info-west = W

## Document reader

read-title = Orbit-Read
read-empty = No document open.
    Drop a PDF, Markdown or text file here, or choose one.
read-open = Open…
read-open-button = Open…
read-filter = Documents (PDF, Markdown, text)
read-failed = Could not open { $name }.
    { $reason }
read-encrypted = { $name } is protected by a password.
read-wrong-password = Wrong password for { $name }.
read-password = Password
read-sidebar = Outline and pages
read-page = Go to page
read-of-pages = of { $total }
read-zoom-out = Zoom out
read-zoom-in = Zoom in
read-fit = Fit to width
read-search = Search the document
read-search-placeholder = Search…
read-search-previous = Previous
read-search-next = Next
read-search-none = Nothing found
read-search-running = Searching…
read-search-count = { $total } found
read-search-position = { $position } of { $total }
read-search-position-running = { $position } of { $total }…
read-night = Night mode
read-print = Print
read-source = Show the source
read-outline = Outline
read-pages = Pages

## Video and audio player

play-title = Orbit-Play
play-empty = Nothing playing.
    Drop a video, a song or a Flash movie here, or choose a file.
play-open = Open…
play-filter = Video, audio and Flash
play-failed = Could not play { $name }.
    { $reason }
play-no-ruffle = Flash movies need Ruffle (ruffle.rs), in ~/.local/bin or on the PATH.
play-play = Play
play-pause = Pause
play-previous = Previous in the folder
play-next = Next in the folder
play-frame-back = Previous frame
play-frame-forward = Next frame
play-mute = Mute
play-volume = Volume
play-speed = Speed
play-audio-track = Audio track
play-subtitles = Subtitles
play-subtitles-off = Off
play-subtitles-load = Load subtitles…
play-track = Track { $n }
play-mono = mono
play-stereo = stereo
play-channels = { $n } channels
play-fullscreen = Full screen
play-leave-fullscreen = Leave full screen
play-key-space = Space

## Text editor

edit-title = Orbit-Edit
edit-untitled = Untitled
edit-untitled-file = untitled.txt
edit-new = New
edit-open = Open…
edit-open-folder = Open folder…
edit-save = Save
edit-save-as = Save as…
edit-sidebar = Folder
edit-no-folder = No folder open
edit-quick-open = Go to file…
edit-close-tab = Close
edit-close-question = Save the changes to { $name }?
edit-close-detail = If you don't save, the changes are lost.
edit-cancel = Cancel
edit-discard = Don't save
edit-open-failed = Could not open { $name }: { $reason }
edit-save-failed = Could not save: { $reason }
edit-find = Find
edit-find-previous = Previous
edit-find-next = Next
edit-find-case = Match case
edit-find-words = Whole words
edit-find-regex = Regular expression
edit-find-close = Close the search
edit-find-none = Nothing found
edit-find-count = { $total } found
edit-find-position = { $position } of { $total }
edit-replace-with = Replace with
edit-replace = Replace
edit-replace-all = Replace all
edit-replaced = { $n } replaced
edit-goto = Go to line
edit-goto-placeholder = Line
edit-status-position = Ln { $line }, Col { $column }
edit-status-spaces = Spaces: { $n }
edit-status-tabs = Tab width: { $n }
edit-indent-spaces = Indent with spaces
edit-indent-width = Width
edit-plain-text = Plain text

## File manager

files-title = Orbit-Files
files-back = Back
files-forward = Forward
files-up = Up a folder
files-filter = Filter
files-hidden = Hidden files
files-split = Two panes
files-view-list = List
files-view-grid = Icons
files-view-columns = Columns
files-column-name = Name
files-column-size = Size
files-column-modified = Modified
files-column-kind = Type
files-status-loading = Reading… { $n } items
files-status-items = { $n } items
files-status-selected = { $n } of { $total } selected ({ $size })
files-cannot-read = Could not read the folder: { $reason }
files-preview-empty = Empty folder
files-preview-link = link
files-places = Places
files-home = Home
files-trash = Trash
files-computer = Computer
files-bookmarks = Bookmarks
files-devices = Devices
files-network = Network
files-connect = Connect to server…
files-eject = Eject
files-open = Open
files-open-with = Open with
files-open-tab = Open in a new tab
files-cut = Cut
files-copy = Copy
files-paste = Paste
files-copy-path = Copy the path
files-rename-menu = Rename…
files-trash-menu = Move to the trash
files-delete-menu = Delete for good…
files-new-folder = New folder
files-new-folder-name = New folder
files-compress-menu = Compress…
files-compress = Compress
files-extract = Extract here
files-archive = archive
files-cancel = Cancel
files-delete = Delete
files-delete-question = Delete { $n } items for good?
files-delete-detail = They do not go to the trash and cannot be recovered.
files-job-copy = Copying { $n } items
files-job-move = Moving { $n } items
files-job-trash = Moving { $n } items to the trash
files-job-delete = Deleting { $n } items
files-job-compress = Compressing { $name }
files-job-extract = Extracting { $name }
files-job-stop = Stop
files-job-errors = Some items could not be processed
files-into-itself = { $name } cannot go into itself
files-conflict = { $name } is already here
files-conflict-detail = What should happen to it?
files-conflict-skip = Skip
files-conflict-skip-all = Skip all
files-conflict-keep = Keep both
files-conflict-replace = Replace
files-conflict-replace-all = Replace all
files-rename = Rename
files-rename-many = Rename { $n } items
files-rename-replace = Find and replace
files-rename-number = Number
files-rename-insert = Insert text
files-rename-case = Upper and lower case
files-rename-find = Find
files-rename-with = Replace with
files-rename-pattern = Pattern: Trip ### (* = current name)
files-rename-before = Before the name
files-rename-after = After the name
files-rename-regex = Regular expression
files-rename-match-case = Match case
files-rename-apply = Rename
files-rename-clash = Some names repeat or are invalid
files-rename-bad-regex = Invalid expression: { $reason }
files-rename-exists = already exists
files-case-lower = lower case
files-case-upper = UPPER CASE
files-case-title = Title Case
files-action-terminal = Open a terminal here
files-copy-to = Copy to
files-move-to = Move to
files-choose = Choose
files-help-title = Keys
files-help-help = These keys
files-help-rename = Rename (many at once: bulk rename)
files-help-filter = Find in this folder
files-help-edit = Edit: text and Markdown in Orbit-Edit
files-help-copy = Copy to the other pane, or to a folder you choose
files-help-move = Move to the other pane, or to a folder you choose
files-help-new-folder = New folder
files-help-trash = Move to the trash
files-help-delete = Delete for good
files-help-view = View: Orbit-View, Orbit-Read or Orbit-Play, by the kind of file
files-help-split = Two panes
files-help-fullscreen = Full screen
files-help-terminal = A terminal in this folder
files-help-toggle = Select or unselect one, keeping the rest
files-help-range = Select a run, from where it started
files-help-mark = Mark and go on
files-help-all = Select all
files-help-invert = Invert the selection
files-help-clear = Clear the marks, or close the search
files-help-clipboard = Copy, cut, paste
files-help-reload = Read the folder again
files-help-hidden = Hidden files
files-help-path = Type a path or smb://…
files-help-views = List, icons, columns
files-help-tabs = New tab, close tab
files-help-other-pane = The other pane
files-help-history = Back, forward, up
files-help-vim = Down, up, out, in; first, last

# Orbit-Pathfinder
pathfinder-title = Orbit-Pathfinder
pathfinder-back = Back
pathfinder-forward = Forward
pathfinder-reload = Reload
pathfinder-hibernate = Hibernate
pathfinder-open-external = Open in the external browser
pathfinder-add = Add a service
pathfinder-menu = Settings
pathfinder-edit-config = Edit settings
pathfinder-reload-config = Reload settings
pathfinder-name = Name
pathfinder-url = Address
pathfinder-group = Group
pathfinder-always-active = Always active (gets notifications)
pathfinder-media = Camera, microphone and calls
pathfinder-add-button = Add
pathfinder-mute = Mute
pathfinder-unmute = Unmute
pathfinder-clear = Clear data (sign out)
pathfinder-clear-confirm = Delete the cookies, logins and cache of { $name }?
pathfinder-remove = Remove
pathfinder-remove-confirm = Remove { $name } from the sidebar? Its data stays on disk.
pathfinder-cancel = Cancel
pathfinder-empty = No services yet. Use + in the sidebar to add one.
pathfinder-group-n = Group { $n } ({ $first }…)
pathfinder-new-group = New group
pathfinder-profile = Profile
pathfinder-profile-hint = optional: same name = same login (e.g. google)
pathfinder-clear-confirm-shared = Delete the cookies, logins and cache of { $name }? The login is shared: { $others } are signed out too.
pathfinder-fill = Fill login from Bitwarden (Ctrl+Shift+L)
pathfinder-bw-missing = The Bitwarden CLI (bw) is not installed.
pathfinder-bw-login = Log in to Bitwarden once in a terminal: bw login
pathfinder-bw-master = Bitwarden master password
pathfinder-bw-unlock = Unlock
pathfinder-bw-wrong = Wrong master password.
pathfinder-bw-none = No Bitwarden login for this site.
pathfinder-import-title = Import passwords into Bitwarden…
pathfinder-import-intro = The passwords these browsers saved go to your Bitwarden vault, into an "Importado do …" folder per browser. Those the vault already has are skipped.
pathfinder-import-source = { $name }: { $count } passwords
pathfinder-import-nothing = No browser with saved passwords was found.
pathfinder-import-firefox-pw = Firefox primary password (only if it has one)
pathfinder-import-go = Import
pathfinder-import-working = Importing…
pathfinder-import-done = { $added } passwords added to Bitwarden; { $already } were already in the vault.
pathfinder-import-failed = The import failed: { $error }
pathfinder-import-firefox-needed = Firefox: the Firefox primary password is needed (or is wrong).
pathfinder-edit = Edit…
pathfinder-save = Save
pathfinder-move-up = Move up
pathfinder-move-down = Move down
pathfinder-icon = Icon
pathfinder-icon-hint = empty = the site's; or an image address, a file
pathfinder-separator-below = Separator below
pathfinder-separator-remove = Remove separator
pathfinder-add-service = Add a service…
pathfinder-add-separator = Add a separator
pathfinder-group-empty = Group { $n } (empty)
pathfinder-browser = Browser
pathfinder-home = Home page (Alt+Home)
pathfinder-address-hint = Type an address or search (Ctrl+L)
pathfinder-unread = { $count ->
    [one] 1 unread
   *[other] { $count } unread
}
pathfinder-new-tab = New tab (Ctrl+T)
pathfinder-close-tab = Close tab (Ctrl+W)
pathfinder-cert-title = Untrusted certificate
pathfinder-cert-detail = { $host } is an address on your local network and showed a certificate from no known authority — usual for Proxmox, routers and NAS, which sign their own. Trust it only if this is your device.
pathfinder-cert-changed = ⚠️ The certificate of { $host } CHANGED since you trusted it. It may be the device reinstalled — or someone posing as it.
pathfinder-cert-facts = Issued to: { $subject }
    Valid until: { $until }
    SHA-256: { $sha256 }
pathfinder-cert-trust = Trust this certificate
pathfinder-error-load = The page could not be opened
pathfinder-error-tls = Invalid certificate
pathfinder-error-tls-detail = This site's certificate is not valid. Pathfinder does not open public sites with an invalid certificate — someone may be posing as it.
pathfinder-downloaded = Download finished: { $name }

## Orbit-Notify, and the panel's bell

notify-title = Notifications
notify-dnd = Do not disturb
notify-clear = Clear all
notify-clear-group = Clear these
notify-empty = No notifications
notifications-none = none new
notifications-new = { $count ->
    [one] 1 new
   *[other] { $count } new
}
notifications-dnd = do not disturb
notifications-on = notifications on
notifications-off = no server
notifications-tooltip = Click: notification centre · Right click: options
notifications-dnd-switch = Do not disturb
notifications-open = Notification centre

## Bluetooth and monitors on the panel

bluetooth-name = Bluetooth
bluetooth-none = no adapter
bluetooth-off = off
bluetooth-on = on
bluetooth-connected = connected
bluetooth-battery = battery { $percent }%
bluetooth-devices = { $count } connected
bluetooth-paired-none = No paired devices
bluetooth-tooltip = Click: power and connect · Right click: manager
monitors-count = { $count ->
    [one] 1 monitor
   *[other] { $count } monitors
}
monitors-above = { $name } above
monitors-below = { $name } below
monitors-left = { $name } on the left
monitors-right = { $name } on the right
monitors-put-above = Above { $main }
monitors-put-below = Below { $main }
monitors-put-left = Left of { $main }
monitors-put-right = Right of { $main }
monitors-single = Only one monitor
monitors-editor-hint = Drag a monitor beside another
monitors-apply = Apply
monitors-tooltip = Click: quick layout · Right click: editor

## Editing the panel

panel-edit = Edit panel
panel-edit-done = Done editing
panel-add = Add module
panel-size-mini = Mini
panel-size-normal = Normal
panel-size-full = Full
panel-move-up = Move up
panel-move-down = Move down
panel-part-start = At the top
panel-part-center = In the middle
panel-part-end = At the bottom
panel-remove = Remove from the panel
panel-module-menu = Menu
panel-module-windows = Window list
panel-module-clock = Clock
panel-module-calendar = Calendar
panel-module-weather = Weather
panel-module-forecast = Forecast
panel-module-tray = System tray
panel-module-volume = Volume
panel-module-brightness = Brightness
panel-module-network = Network
panel-module-keyboard = Keyboard
panel-module-battery = Battery
panel-module-notifications = Notifications
panel-module-bluetooth = Bluetooth
panel-module-monitors = Monitors
panel-module-workspaces = Workspaces
panel-module-session = Session
panel-module-spectrum = Spectrum
panel-module-aigent = Aigent
panel-module-cpu = CPU
panel-module-ram = RAM
panel-module-gpu = GPU
panel-module-lan = LAN
panel-module-disk = Disk
panel-filesystem = Filesystem

# Disk module (orbit-panel)
disk-total = TOTAL
disk-used = USED
disk-free = FREE

# Orbit-Term
term-history = Outputs
term-no-shell = The shell could not start
term-starting = The shell is still starting
term-running = Running { $command }…
term-interactive = { $command } — in the terminal
term-exit = exit { $code }
term-stderr = wrote errors
term-from-history = { $command }  ·  { $time }  ·  { $cwd }

## Login and lock screen

greet-password = Password
greet-checking = Checking…
greet-wrong = Wrong password
greet-wait = Try again in { $seconds } s
greet-suspend = suspend

## The polkit password dialog

polkit-title = Authentication required
polkit-authenticate = Authenticate
polkit-cancel = Cancel
polkit-details = Details

## The run line (Mod+R)

run-title = Run
run-placeholder = Command to run
run-not-found = { $program }: no such program

## The keyring's password prompt

prompter-title = Keyring
prompter-continue = Continue
prompter-again = Password again
prompter-mismatch = The passwords do not match

## The aigent module (aigent.rs)

aigent-offline = Orbit-Aigent is not running
aigent-agents = Agents
aigent-cancel = Cancel
aigent-yes = Yes
aigent-no = No
