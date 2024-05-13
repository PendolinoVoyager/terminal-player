# terminal-player
Play video (mp4) to ASCII in your terminal when you lose your desktop somewhere.
## Requirments
Your terminal must support ANSI escape codes.
## Usage
First build it using cargo or rust compiler.
Then:
\<binary name\> -w \<width\> \<filename\> will start the program.
## Additional notes
It should support a large mp4, but I haven't checked for memory usage over time. \
It mallocs strings every frame so, dunno.
