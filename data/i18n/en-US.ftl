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

## Keyboard

keyboard-next = { $name } — click for the next one:

## Battery

battery-none = no battery
battery-uptime = up for { $uptime }
battery-tooltip = battery { $percent }%, { $state }{ $remaining }
battery-remaining = {" "}— { $time } left
battery-charging = charging
battery-discharging = discharging
battery-full = charged
battery-pending = waiting to charge

## Weather

weather-tooltip = { $description } in { $place }
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

season-winter = winter
season-spring = spring
season-summer = summer
season-autumn = autumn

## Dock

dock-open = Open
dock-recent = Recent

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
