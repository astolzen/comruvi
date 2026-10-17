# Project: „Rustview“  
Folder: F:\sync\ast\git\rustview  
Code a portable Image Viewer in Rust which is optimized to display AI generated images with the generation data  
The initial Version will work on Windows, but once we have a stable release of it, I want to port it to MacOS and Linux as well.  

## Purpose:  
The main focus of this image viewer is to visualize not only an image, but the exif-Information as well, specifically the AI Image generation data (Model used, positive/negative Prompt, used Loras etc..)

## Main Window:  
The main windows of the viewer is split in three resizable columns that can be turned on and of through buttons:  
- Left pane: shows the filesystem and subdirectories (as tree), user can click into folders to get the content in the main view
  - Middle pane: Main view with at least two switchable modes  
    - Thumbnails: shows all Images of the folder as thumbnails.  
    - Filmstrip: shows one image as „resize to fit“ in the upper part of the pane and a thumbnails from left to right underneat it  
- Right pane: Shows the exif info, mainly the AI generation data (Model, Loras, lora weights, positive/negative Prompt etc..) The pane is read only but must allow the user to easyly copy parts of the generation data to the clipboard.

## Thumbnails  
The main view will primarily display the „thumbnail“ version of an image. Thumbnails will be stored in a subdirectory „.thumbnails“ in the png format with a maximum size of 160x160. This will be compatible to the thumbnail system of other viewers like GeeQie (https://github.com/BestImageViewer/geeqie). If there are no thumbnails available and/or the .thumbnails directory is missing, the program will create new thumbnails as png in the size 160x160 (later version should allow the user to set the thumbnail resolution). Also there should be an option to „recreate thumbnails“.  

## seperate Viewer  
Besides the view funtion in the middle pane, the user can double click an image. This will open a separate, rezizable window with the image, intended to be used on systems with multiple screens. The main window with the three panes stays open and usable, while the full image view panel exists and can be closed by the user without shutting the main program down. if the user (single) clicks another image in the main view, while the viewer window is open, it will update to show that image.

## Exif:  
The problem with the exif-information of the AI generation data is: it can be stored with different tags depending on the used comfyui nodes or ai generation tools. The viewer should mailny support the format of comfyui.   
The subfolder „examples“ contains a series of images: Some do not have any AI information (real photos) others have AI Information in them. Figure out, what relevant info is stored where. There also is a .“thumbnails“ folder underneath examples with some already created thumbnails. Use the nameing scheme used here for the thumbnail creation  

## File formats:  
For the first release, it is sufficient to support the formats jpg and png. Maybe other formats will follow.  

## Tasks:  
Create the program in rust and document code and progress. Feel free to explore other open source image viewers and tools like geeqie or gimp. If you use code from these other projects, mention it in the documentation, so we can credit the developers.  
The program itself will not overwrite any of the original png or jpg images — the only write can be done to the thumbnails.    
