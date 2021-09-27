# hinoki
Procedure Terrain Generation. This project is a toy project of mine that generate an infinite world with diversity of terrains, biomes, weathers, animals. It is inspired by some Youtube series by [Sebastian Lague](https://www.youtube.com/channel/UCmtyQOKKmrMVaKuRXz02jbQ), [CodeParade](https://www.youtube.com/c/CodeParade), [SimonDev](https://www.youtube.com/channel/UCEwhtpXrg5MmwlH04ANpL8A). These video are either using heightmap to generate mountains or marching cubes to generate caves but not both. I want to have my own take at it, generate an infinite world that can contains both and most of all fun to explore. I want to learn Rust so I chose it as my experiment programming language.

![Preview](https://github.com/bachdx96/hinoki/raw/master/.github/preview.gif)

## Implemented

✅ Simple terrain generation

✅ Using quad tree to adjust the size of rendered chunk

✅ Adjust LOD base on distance to camera

✅ Multithreaded terrain generation

## To be implemented

✅ Better stitching between different chunks

✅ Better render shader

✅ Using integer mixed with floating point noise function for high precision when floating point number grows too large

✅ Biomes, animals, weather, ...