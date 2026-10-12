// The server-side layout of `level_03_evn`, the only map srp offers.
//
// Generated from the original server's map project, `projects/level_03_evn/
// server_project.lua` inside the game's `resources.db` (read with the
// srp-survarium/vostok-resources-db unpacker); see WORK.md for the steps.
// Each table is in the project's `point_id` / `id` order:
//
//   * `victory_item_spawners`: where batteries appear;
//   * `respawn_points`: per team; `priority = 2` points stand at the team's
//     base and `priority = 1` points are the "safe_respawn" fallbacks further
//     back. Each lists ally and enemy zones (collision boxes): a body-sized
//     box on the point itself and, for base points, one over the whole base;
//   * `victory_items_containers`: each team's battery base. A player uses it
//     through its collision geometry (a box; the mesh scale is the half
//     extents), not at the container's own position.
//
// Yaws are what the client reads back from the editor rotation
// (`get_angles(rotation_zxy).y` of the xyz `create_rotation`).

use survarium::player_profile::raw::game_team_id;
use vostok::math::float3;

/// An axis-aligned box: a collision geometry's single box mesh (the mesh
/// scale times the object scale gives the half extents).
#[derive(Clone, Copy, Debug)]
pub struct Zone {
    pub center: float3,
    pub half_extents: float3,
}

impl Zone {
    /// Whether a standing player's body, a vertical segment from `feet` up
    /// `height` metres, overlaps the box.
    pub fn touches_body(&self, feet: float3, height: f32) -> bool {
        let Zone {
            center,
            half_extents: half,
        } = *self;
        (feet.x - center.x).abs() <= half.x
            && (feet.z - center.z).abs() <= half.z
            && feet.y <= center.y + half.y
            && feet.y + height >= center.y - half.y
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RespawnPoint {
    pub team: game_team_id,
    /// Higher is preferred: 2 at the team's base, 1 for the "safe_respawn"
    /// points behind it.
    pub priority: u8,
    pub position: float3,
    pub yaw: f32,
    /// A teammate in any of these keeps the point closed.
    pub ally_zones: &'static [Zone],
    /// An enemy in any of these keeps the point closed.
    pub enemy_zones: &'static [Zone],
}

#[derive(Clone, Copy, Debug)]
pub struct BatteryBase {
    /// The container id the client knows the base by.
    pub id: u8,
    pub team: game_team_id,
    pub use_zone_center: float3,
    pub use_zone_half_extents: float3,
}

/// A battery's own use zone: a sphere of this radius around its position
/// (`gameplay/victory_items/default.lua`).
pub const BATTERY_USE_RADIUS: f32 = 1.0;

pub const BATTERY_SPAWNS: [float3; 26] = [
    float3 {
        x: -19.012,
        y: 10.675,
        z: -12.495,
    }, // victory_item_spawner(21)
    float3 {
        x: -26.245,
        y: 6.731,
        z: -3.284,
    }, // victory_item_spawner(13)
    float3 {
        x: 52.38,
        y: 0.413,
        z: -56.971,
    }, // victory_item_spawner(1)
    float3 {
        x: 27.685,
        y: 1.139,
        z: -63.41,
    }, // victory_item_spawner(6)
    float3 {
        x: -29.959,
        y: 10.656,
        z: -17.13,
    }, // victory_item_spawner(19)
    float3 {
        x: -18.836,
        y: 14.406,
        z: -17.809,
    }, // victory_item_spawner(24)
    float3 {
        x: 43.297,
        y: -2.236,
        z: -60.805,
    }, // victory_item_spawner
    float3 {
        x: 47.384,
        y: 1.313,
        z: -52.79,
    }, // victory_item_spawner(2)
    float3 {
        x: -51.651,
        y: 5.536,
        z: 25.436,
    }, // victory_item_spawner(9)
    float3 {
        x: 24.107,
        y: 1.437,
        z: -42.912,
    }, // victory_item_spawner(8)
    float3 {
        x: -3.531,
        y: 10.131,
        z: 19.559,
    }, // victory_item_spawner(26)
    float3 {
        x: -13.292,
        y: 6.741,
        z: -6.625,
    }, // victory_item_spawner(12)
    float3 {
        x: -49.399,
        y: 5.958,
        z: 24.162,
    }, // victory_item_spawner(11)
    float3 {
        x: -24.799,
        y: 10.656,
        z: -15.419,
    }, // victory_item_spawner(22)
    float3 {
        x: -25.435,
        y: 10.656,
        z: -5.862,
    }, // victory_item_spawner(20)
    float3 {
        x: -19.776,
        y: 5.956,
        z: -29.446,
    }, // victory_item_spawner(17)
    float3 {
        x: 32.752,
        y: 1.333,
        z: -66.365,
    }, // victory_item_spawner(5)
    float3 {
        x: 42.548,
        y: 1.098,
        z: -47.558,
    }, // victory_item_spawner(3)
    float3 {
        x: 25.315,
        y: 1.555,
        z: -39.824,
    }, // victory_item_spawner(7)
    float3 {
        x: -22.312,
        y: 7.512,
        z: -26.054,
    }, // victory_item_spawner(16)
    float3 {
        x: -35.227,
        y: 6.246,
        z: -4.506,
    }, // victory_item_spawner(14)
    float3 {
        x: 37.655,
        y: 1.289,
        z: -68.921,
    }, // victory_item_spawner(4)
    float3 {
        x: -61.521,
        y: 6.243,
        z: 32.048,
    }, // victory_item_spawner(10)
    float3 {
        x: -50.007,
        y: 9.951,
        z: 5.232,
    }, // victory_item_spawner(25)
    float3 {
        x: -12.756,
        y: 10.656,
        z: -18.833,
    }, // victory_item_spawner(18)
    float3 {
        x: -24.355,
        y: 14.406,
        z: -10.635,
    }, // victory_item_spawner(23)
];

pub const RESPAWN_POINTS: [RespawnPoint; 43] = [
    // logic/team2/respawn_point(4); ally zones team2/respawn_colision(3); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 48.593,
            y: 0.433,
            z: 16.238,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 48.559,
                y: 0.554,
                z: 16.277,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(2); ally zones team2/respawn_colision(1); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 40.903,
            y: 0.46,
            z: 22.131,
        },
        yaw: -0.201,
        ally_zones: &[Zone {
            center: float3 {
                x: 40.961,
                y: 0.567,
                z: 22.048,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(6); ally zones team2/respawn_colision(5); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 37.725,
            y: 0.472,
            z: 20.503,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 37.748,
                y: 0.47,
                z: 20.5,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(7); ally zones team2/respawn_colision(6); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 38.883,
            y: 0.445,
            z: 14.502,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 38.949,
                y: 0.445,
                z: 14.471,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(9); ally zones team2/team_2_enemy_respawn_collision, team2/respawn_colision(8); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 40.447,
            y: 0.464,
            z: 16.112,
        },
        yaw: 1.447,
        ally_zones: &[
            Zone {
                center: float3 {
                    x: 38.694,
                    y: 2.532,
                    z: 13.649,
                },
                half_extents: float3 {
                    x: 12.167,
                    y: 1.942,
                    z: 11.818,
                },
            },
            Zone {
                center: float3 {
                    x: 40.475,
                    y: 0.464,
                    z: 16.128,
                },
                half_extents: float3 {
                    x: 0.37,
                    y: 1.634,
                    z: 0.44,
                },
            },
        ],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(8); ally zones team2/respawn_colision(7); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 49.236,
            y: 0.433,
            z: 17.96,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 49.252,
                y: 0.554,
                z: 17.962,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/safe_respawn/respawn_point(10); ally zones team2/safe_respawn/respawn_colision(9); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 65.521,
            y: 0.914,
            z: 20.86,
        },
        yaw: 3.111,
        ally_zones: &[Zone {
            center: float3 {
                x: 65.361,
                y: 0.912,
                z: 21.118,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(2); ally zones team2/safe_respawn/respawn_colision(1); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 78.073,
            y: 0.924,
            z: 10.981,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 78.048,
                y: 0.928,
                z: 10.913,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(5); ally zones team2/safe_respawn/respawn_colision(4); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 65.121,
            y: 0.912,
            z: 16.65,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 65.213,
                y: 1.072,
                z: 16.66,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(11); ally zones team2/safe_respawn/respawn_colision(10); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 77.04,
            y: 0.912,
            z: 8.231,
        },
        yaw: 2.997,
        ally_zones: &[Zone {
            center: float3 {
                x: 77.077,
                y: 1.04,
                z: 8.18,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(1); ally zones team2/safe_respawn/respawn_colision; enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 77.857,
            y: 0.924,
            z: 9.413,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 77.897,
                y: 0.938,
                z: 9.394,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(8); ally zones team2/safe_respawn/respawn_colision(7); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 61.196,
            y: 0.897,
            z: 5.222,
        },
        yaw: -0.090,
        ally_zones: &[Zone {
            center: float3 {
                x: 61.184,
                y: 0.873,
                z: 5.057,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(7); ally zones team2/safe_respawn/respawn_colision(6); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 68.577,
            y: 0.988,
            z: 10.167,
        },
        yaw: 2.922,
        ally_zones: &[Zone {
            center: float3 {
                x: 68.597,
                y: 0.938,
                z: 10.123,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(3); ally zones team2/safe_respawn/respawn_colision(2); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 76.723,
            y: 0.912,
            z: 15.363,
        },
        yaw: -0.109,
        ally_zones: &[Zone {
            center: float3 {
                x: 76.783,
                y: 0.912,
                z: 15.409,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(9); ally zones team2/safe_respawn/respawn_colision(8); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 63.45,
            y: 0.942,
            z: 2.819,
        },
        yaw: -0.090,
        ally_zones: &[Zone {
            center: float3 {
                x: 63.406,
                y: 0.954,
                z: 2.749,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(6); ally zones team2/safe_respawn/respawn_colision(5); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 71.327,
            y: 0.912,
            z: 21.082,
        },
        yaw: -3.095,
        ally_zones: &[Zone {
            center: float3 {
                x: 71.33,
                y: 0.912,
                z: 21.103,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/safe_respawn/respawn_point(4); ally zones team2/safe_respawn/respawn_colision(3); enemy zones none
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 1,
        position: float3 {
            x: 65.234,
            y: 1.057,
            z: 9.439,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 65.315,
                y: 1.121,
                z: 9.394,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team2/respawn_point(10); ally zones team2/respawn_colision(9); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 40.225,
            y: 0.423,
            z: 18.239,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 40.197,
                y: 0.423,
                z: 18.273,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(5); ally zones team2/respawn_colision(4); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 36.907,
            y: 0.436,
            z: 14.439,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 36.923,
                y: 0.675,
                z: 14.42,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(1); ally zones team2/respawn_colision; enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 48.227,
            y: 0.435,
            z: 14.433,
        },
        yaw: 1.447,
        ally_zones: &[Zone {
            center: float3 {
                x: 48.23,
                y: 0.554,
                z: 14.42,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team2/respawn_point(3); ally zones team2/respawn_colision(2); enemy zones team2/team_2_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_2,
        priority: 2,
        position: float3 {
            x: 39.197,
            y: 0.46,
            z: 22.131,
        },
        yaw: -0.201,
        ally_zones: &[Zone {
            center: float3 {
                x: 39.236,
                y: 0.567,
                z: 22.048,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: 38.694,
                y: 2.532,
                z: 13.649,
            },
            half_extents: float3 {
                x: 12.167,
                y: 1.942,
                z: 11.818,
            },
        }],
    },
    // logic/team1/respawn_point; ally zones team1/respawn_colision; enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -30.552,
            y: 0.505,
            z: -80.514,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -30.512,
                y: 0.484,
                z: -80.631,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(3); ally zones team1/respawn_colision(3); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -36.052,
            y: 0.512,
            z: -80.579,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -36.034,
                y: 0.519,
                z: -80.631,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/safe_reswapn/respawn_point(3); ally zones team1/safe_reswapn/respawn_colision(6); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -37.864,
            y: 0.863,
            z: -101.233,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -37.836,
                y: 0.879,
                z: -101.078,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(2); ally zones team1/safe_reswapn/respawn_colision(8); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -37.493,
            y: 0.863,
            z: -106.277,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -37.46,
                y: 1.079,
                z: -106.264,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(8); ally zones team1/safe_reswapn/respawn_colision(2); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -49.8,
            y: 0.843,
            z: -99.709,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -49.825,
                y: 0.843,
                z: -99.567,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(9); ally zones team1/safe_reswapn/respawn_colision(4); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -51.09,
            y: 0.843,
            z: -93.553,
        },
        yaw: -1.772,
        ally_zones: &[Zone {
            center: float3 {
                x: -51.107,
                y: 0.879,
                z: -93.592,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(7); ally zones team1/safe_reswapn/respawn_colision(3); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -47.319,
            y: 0.843,
            z: -99.681,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -47.349,
                y: 0.843,
                z: -99.567,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(4); ally zones team1/safe_reswapn/respawn_colision(7); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -35.884,
            y: 0.978,
            z: -101.305,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -35.817,
                y: 0.879,
                z: -101.078,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(1); ally zones team1/safe_reswapn/respawn_colision(9); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -35.085,
            y: 0.863,
            z: -106.277,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -35.108,
                y: 0.979,
                z: -106.168,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(11); ally zones team1/safe_reswapn/respawn_colision(10); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -33.399,
            y: 0.863,
            z: -106.277,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -33.314,
                y: 0.879,
                z: -106.198,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(10); ally zones team1/safe_reswapn/respawn_colision(5); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -39.174,
            y: 0.863,
            z: -93.781,
        },
        yaw: -1.522,
        ally_zones: &[Zone {
            center: float3 {
                x: -39.202,
                y: 0.879,
                z: -93.726,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(5); ally zones team1/safe_reswapn/respawn_colision; enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -50.628,
            y: 0.843,
            z: -105.438,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -50.634,
                y: 0.843,
                z: -105.398,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/safe_reswapn/respawn_point(6); ally zones team1/safe_reswapn/respawn_colision(1); enemy zones none
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 1,
        position: float3 {
            x: -48.44,
            y: 0.843,
            z: -105.438,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -48.44,
                y: 0.843,
                z: -105.398,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[],
    },
    // logic/team1/respawn_point(10); ally zones team1/respawn_colision(10); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -30.707,
            y: 0.505,
            z: -70.676,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -30.657,
                y: 0.512,
                z: -70.718,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(8); ally zones team1/respawn_colision(8); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -44.24,
            y: 0.512,
            z: -71.592,
        },
        yaw: -1.650,
        ally_zones: &[Zone {
            center: float3 {
                x: -44.27,
                y: 0.705,
                z: -71.595,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(7); ally zones team1/respawn_colision(7); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -33.108,
            y: 0.505,
            z: -70.676,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -33.1,
                y: 0.512,
                z: -70.718,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(6); ally zones team1/respawn_colision(6); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -33.28,
            y: 0.505,
            z: -76.109,
        },
        yaw: -1.575,
        ally_zones: &[Zone {
            center: float3 {
                x: -33.18,
                y: 0.512,
                z: -76.084,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(9); ally zones team1/respawn_colision(9); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -43.965,
            y: 0.512,
            z: -75.591,
        },
        yaw: -1.650,
        ally_zones: &[Zone {
            center: float3 {
                x: -43.963,
                y: 0.705,
                z: -75.655,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(1); ally zones team1/respawn_colision(1); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -31.989,
            y: 0.512,
            z: -80.615,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -31.943,
                y: 0.519,
                z: -80.631,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(5); ally zones team1/respawn_colision(5); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -35.788,
            y: 0.512,
            z: -74.422,
        },
        yaw: -2.018,
        ally_zones: &[Zone {
            center: float3 {
                x: -35.718,
                y: 0.519,
                z: -74.42,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(4); ally zones team1/respawn_colision(4); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -35.788,
            y: 0.512,
            z: -76.424,
        },
        yaw: 0.0,
        ally_zones: &[Zone {
            center: float3 {
                x: -35.687,
                y: 0.519,
                z: -76.521,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
    // logic/team1/respawn_point(2); ally zones team1/respawn_colision(2); enemy zones team1/team_1_enemy_respawn_collision
    RespawnPoint {
        team: game_team_id::team_1,
        priority: 2,
        position: float3 {
            x: -35.845,
            y: 0.512,
            z: -70.771,
        },
        yaw: -2.355,
        ally_zones: &[Zone {
            center: float3 {
                x: -35.858,
                y: 0.503,
                z: -70.816,
            },
            half_extents: float3 {
                x: 0.37,
                y: 1.634,
                z: 0.44,
            },
        }],
        enemy_zones: &[Zone {
            center: float3 {
                x: -31.318,
                y: 2.126,
                z: -73.435,
            },
            half_extents: float3 {
                x: 14.281,
                y: 1.493,
                z: 8.343,
            },
        }],
    },
];

pub const BATTERY_BASES: [BatteryBase; 2] = [
    BatteryBase {
        id: 0,
        team: game_team_id::team_2,
        use_zone_center: float3 {
            x: 49.851,
            y: 1.229,
            z: 14.625,
        },
        use_zone_half_extents: float3 {
            x: 0.689,
            y: 0.812,
            z: 0.76,
        },
    }, // logic/team2/team_2_vic_collision
    BatteryBase {
        id: 1,
        team: game_team_id::team_1,
        use_zone_center: float3 {
            x: -48.7,
            y: 1.148,
            z: -73.263,
        },
        use_zone_half_extents: float3 {
            x: 0.702,
            y: 0.658,
            z: 0.772,
        },
    }, // logic/team1/team_1_vic_collision
];

/// The base of `team`, if the map has one.
pub fn battery_base_of(team: game_team_id) -> Option<&'static BatteryBase> {
    BATTERY_BASES.iter().find(|base| base.team == team)
}

pub fn battery_base(id: u8) -> Option<&'static BatteryBase> {
    BATTERY_BASES.iter().find(|base| base.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_team_has_a_base_and_base_respawn_points() {
        for team in [game_team_id::team_1, game_team_id::team_2] {
            assert!(battery_base_of(team).is_some(), "{team:?}");
            assert!(
                RESPAWN_POINTS
                    .iter()
                    .any(|point| point.team == team && point.priority == 2),
                "{team:?}"
            );
        }
        assert_eq!(battery_base(1).unwrap().team, game_team_id::team_1);
        assert_eq!(battery_base(0).unwrap().team, game_team_id::team_2);
    }

    #[test]
    fn a_player_on_each_point_stands_in_its_zones() {
        const BODY_HEIGHT: f32 = 1.8;
        for (index, point) in RESPAWN_POINTS.iter().enumerate() {
            assert!(
                point
                    .ally_zones
                    .iter()
                    .any(|zone| zone.touches_body(point.position, BODY_HEIGHT)),
                "point {index} is outside its ally zones"
            );
            // Base points close while an enemy is in the base; safe points
            // have no enemy zone.
            assert_eq!(point.enemy_zones.is_empty(), point.priority == 1, "{index}");
            for zone in point.enemy_zones {
                // The zone starts above the floor: only the body reaches it.
                assert!(zone.center.y - zone.half_extents.y > point.position.y);
                assert!(zone.touches_body(point.position, BODY_HEIGHT), "{index}");
            }
        }
    }

    #[test]
    fn a_body_touches_a_zone_only_within_its_footprint_and_height() {
        let zone = Zone {
            center: float3 {
                x: 0.0,
                y: 2.0,
                z: 0.0,
            },
            half_extents: float3 {
                x: 1.0,
                y: 1.0,
                z: 1.0,
            },
        };
        let feet = |x, y| float3 { x, y, z: 0.0 };
        assert!(zone.touches_body(feet(0.0, 0.5), 1.8), "head inside");
        assert!(!zone.touches_body(feet(0.0, -0.9), 1.8), "below");
        assert!(!zone.touches_body(feet(0.0, 3.1), 1.8), "above");
        assert!(!zone.touches_body(feet(1.1, 2.0), 1.8), "beside");
    }
}
