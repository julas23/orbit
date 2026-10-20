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
