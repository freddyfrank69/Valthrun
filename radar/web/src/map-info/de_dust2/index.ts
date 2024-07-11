import { LoadedMap } from "..";
import kImageRadar from "./radar.png";
import kImageBuyZones from "./overlay_buyzones.png";

export default {
    mapName: "de_dust2",
    displayName: "Dust 2",

    metaInfo: {
        "resolution": 4.40,

        "offset": {
            "x": 2470,
            "y": 1255
        },

        "floors": []
    },

    overlayBuyzones: kImageBuyZones,
    overlayRadar: kImageRadar,
} satisfies LoadedMap;